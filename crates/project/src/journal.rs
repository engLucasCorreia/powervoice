//! The append-only edit journal (ADR-004 §6).
//!
//! One record per line: `<crc32 hex>\t<compact JSON>\n`, CRC32 over the JSON bytes. Every
//! [`Journal::append`] writes its records with one `write` and `fdatasync`s before returning, so a
//! command that returned `Ok` survives a crash. A torn or corrupt line ends the valid journal
//! ([`parse_journal`]); replay and recovery on top of it live in [`crate::recovery`] (T-301).

use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::io::Write as _;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::fs_util::{from_hex, sync_dir, to_hex, write_all_at};
use crate::history::{Edit, EditOp, Entry, History, LabelParams, MarkerMapping, MarkerOp};
use crate::snapshot::{DocSnapshot, Marker, MarkerId, Piece, Source};
use crate::store::{ChunkId, ChunkLocation};
use crate::{ProjectError, Result};

/// File name of the journal of generation `generation`.
pub fn journal_file_name(generation: u32) -> String {
    format!("journal.{generation}.jsonl")
}

/// Bytes serialized as a lowercase hex string (opaque attachments).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HexBytes(pub Vec<u8>);

impl Serialize for HexBytes {
    fn serialize<S: Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        s.serialize_str(&to_hex(&self.0))
    }
}

impl<'de> Deserialize<'de> for HexBytes {
    fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        from_hex(&s)
            .map(HexBytes)
            .ok_or_else(|| serde::de::Error::custom("invalid hex string"))
    }
}

/// The user file a session was opened from.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceRecord {
    pub path: String,
    pub size_bytes: u64,
    pub mtime_unix_ms: u64,
    pub format: String,
}

/// Where a take goes (ADR-004 §6 `take_begin.mode`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TakeModeRecord {
    New,
    Insert,
    Overwrite,
    Punch,
}

/// A piece: `{"chunk":3,"offset":0,"len":65536}` or `{"silence":480}`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum PieceRecord {
    Chunk {
        chunk: ChunkId,
        offset: u32,
        len: u32,
    },
    Silence {
        silence: u32,
    },
}

impl PieceRecord {
    pub fn from_piece(piece: &Piece) -> Self {
        match piece.source {
            Source::Chunk(chunk) => PieceRecord::Chunk {
                chunk,
                offset: piece.offset,
                len: piece.len,
            },
            Source::Silence => PieceRecord::Silence { silence: piece.len },
        }
    }

    pub fn to_piece(&self) -> Piece {
        match *self {
            PieceRecord::Chunk { chunk, offset, len } => Piece::chunk(chunk, offset, len),
            PieceRecord::Silence { silence } => Piece::silence(silence),
        }
    }
}

/// A marker.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MarkerRecord {
    pub id: u64,
    pub pos: u64,
    pub len: u64,
    pub name: String,
}

impl MarkerRecord {
    pub fn from_marker(m: &Marker) -> Self {
        MarkerRecord {
            id: m.id.0,
            pos: m.pos_samples,
            len: m.len_samples,
            name: m.name.to_string(),
        }
    }

    pub fn to_marker(&self) -> Marker {
        Marker::new(MarkerId(self.id), self.pos, self.len, self.name.as_str())
    }
}

/// How a replace op maps markers (`"shift"` = SPEC-008 §4.2, `"identity"` = no marker moves).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MarkerMappingRecord {
    #[default]
    Shift,
    Identity,
}

impl From<MarkerMapping> for MarkerMappingRecord {
    fn from(m: MarkerMapping) -> Self {
        match m {
            MarkerMapping::Shift => MarkerMappingRecord::Shift,
            MarkerMapping::Identity => MarkerMappingRecord::Identity,
        }
    }
}

impl From<MarkerMappingRecord> for MarkerMapping {
    fn from(m: MarkerMappingRecord) -> Self {
        match m {
            MarkerMappingRecord::Shift => MarkerMapping::Shift,
            MarkerMappingRecord::Identity => MarkerMapping::Identity,
        }
    }
}

/// A piece-table delta.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpRecord {
    Replace {
        at: u64,
        remove_len: u64,
        pieces: Vec<PieceRecord>,
        #[serde(default)]
        mapping: MarkerMappingRecord,
    },
}

/// A marker op.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MarkerOpRecord {
    Add(MarkerRecord),
    Remove { id: u64 },
    Rename { id: u64, name: String },
    Move { id: u64, pos: u64, len: u64 },
}

/// `edit {seq, label_key, label_params?, ops, marker_ops, take?, attachment?}`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EditRecord {
    pub seq: u64,
    pub label_key: String,
    /// ADR-004 Amendment 3: the label's placeholder values (`{"target": "−1"}`).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub label_params: LabelParams,
    #[serde(default)]
    pub ops: Vec<OpRecord>,
    #[serde(default)]
    pub marker_ops: Vec<MarkerOpRecord>,
    /// The take this edit commits (pairs it with its `take_begin`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub take: Option<u32>,
    /// ADR-004 Amendment 1: opaque, stored verbatim.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attachment: Option<HexBytes>,
    /// Set when the take's WAV holds more samples than the committed audio (the chunk store
    /// failed mid-take): the WAV tail is still recoverable, so GC keeps the session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub take_wav_samples: Option<u64>,
}

impl EditRecord {
    pub fn from_edit(seq: u64, edit: &Edit, take: Option<u32>) -> Self {
        EditRecord {
            seq,
            label_key: edit.label_key.clone(),
            label_params: edit.label_params.clone(),
            ops: edit
                .ops
                .iter()
                .map(|op| {
                    let EditOp::Replace {
                        at,
                        remove_len,
                        pieces,
                        mapping,
                    } = op;
                    OpRecord::Replace {
                        at: *at,
                        remove_len: *remove_len,
                        pieces: pieces.iter().map(PieceRecord::from_piece).collect(),
                        mapping: (*mapping).into(),
                    }
                })
                .collect(),
            marker_ops: edit
                .marker_ops
                .iter()
                .map(|op| match op {
                    MarkerOp::Add(m) => MarkerOpRecord::Add(MarkerRecord::from_marker(m)),
                    MarkerOp::Remove(id) => MarkerOpRecord::Remove { id: id.0 },
                    MarkerOp::Rename { id, name } => MarkerOpRecord::Rename {
                        id: id.0,
                        name: name.to_string(),
                    },
                    MarkerOp::Move {
                        id,
                        pos_samples,
                        len_samples,
                    } => MarkerOpRecord::Move {
                        id: id.0,
                        pos: *pos_samples,
                        len: *len_samples,
                    },
                })
                .collect(),
            take,
            attachment: edit.attachment.clone().map(HexBytes),
            take_wav_samples: None,
        }
    }

    /// The edit this record describes (for replay).
    pub fn to_edit(&self) -> Edit {
        Edit {
            label_key: self.label_key.clone(),
            label_params: self.label_params.clone(),
            ops: self
                .ops
                .iter()
                .map(|op| {
                    let OpRecord::Replace {
                        at,
                        remove_len,
                        pieces,
                        mapping,
                    } = op;
                    EditOp::Replace {
                        at: *at,
                        remove_len: *remove_len,
                        pieces: pieces.iter().map(PieceRecord::to_piece).collect(),
                        mapping: (*mapping).into(),
                    }
                })
                .collect(),
            marker_ops: self
                .marker_ops
                .iter()
                .map(|op| match op {
                    MarkerOpRecord::Add(m) => MarkerOp::Add(m.to_marker()),
                    MarkerOpRecord::Remove { id } => MarkerOp::Remove(MarkerId(*id)),
                    MarkerOpRecord::Rename { id, name } => MarkerOp::Rename {
                        id: MarkerId(*id),
                        name: name.as_str().into(),
                    },
                    MarkerOpRecord::Move { id, pos, len } => MarkerOp::Move {
                        id: MarkerId(*id),
                        pos_samples: *pos,
                        len_samples: *len,
                    },
                })
                .collect(),
            attachment: self.attachment.as_ref().map(|a| a.0.clone()),
        }
    }
}

/// A full snapshot (checkpoints).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotRecord {
    pub rev: u64,
    pub audio_rev: u64,
    pub sample_rate_hz: u32,
    pub pieces: Vec<PieceRecord>,
    pub markers: Vec<MarkerRecord>,
}

impl SnapshotRecord {
    pub fn from_snapshot(s: &DocSnapshot) -> Self {
        SnapshotRecord {
            rev: s.rev,
            audio_rev: s.audio_rev,
            sample_rate_hz: s.sample_rate_hz,
            pieces: s.pieces.iter().map(PieceRecord::from_piece).collect(),
            markers: s.markers.iter().map(MarkerRecord::from_marker).collect(),
        }
    }

    pub fn to_snapshot(&self) -> DocSnapshot {
        DocSnapshot {
            rev: self.rev,
            audio_rev: self.audio_rev,
            ..DocSnapshot::new(
                self.sample_rate_hz,
                self.pieces.iter().map(PieceRecord::to_piece).collect(),
                self.markers.iter().map(MarkerRecord::to_marker).collect(),
            )
        }
    }
}

/// One undo/redo stack entry (checkpoints).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EntryRecord {
    pub seq: u64,
    pub label_key: String,
    /// ADR-004 Amendment 3.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub label_params: LabelParams,
    pub audio: bool,
    /// The edit's first affected position (SPEC-008 §2.3's undo/redo cursor rule, Amendment 3).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attachment: Option<HexBytes>,
    pub snapshot: SnapshotRecord,
}

impl EntryRecord {
    fn from_entry(e: &Entry) -> Self {
        EntryRecord {
            seq: e.seq,
            label_key: e.label_key.to_string(),
            label_params: (*e.label_params).clone(),
            audio: e.audio,
            first_at: e.first_at,
            attachment: e.attachment.as_deref().map(|a| HexBytes(a.to_vec())),
            snapshot: SnapshotRecord::from_snapshot(&e.snapshot),
        }
    }

    fn to_entry(&self) -> Entry {
        Entry {
            snapshot: std::sync::Arc::new(self.snapshot.to_snapshot()),
            seq: self.seq,
            label_key: self.label_key.as_str().into(),
            label_params: std::sync::Arc::new(self.label_params.clone()),
            attachment: self.attachment.as_ref().map(|a| a.0.as_slice().into()),
            audio: self.audio,
            first_at: self.first_at,
        }
    }
}

/// `checkpoint`: the full chunk index, stacks, current snapshot and every counter needed to
/// continue exactly (ADR-004 §6).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckpointRecord {
    /// Current seq.
    pub seq: u64,
    pub saved_seq: u64,
    pub next_seq: u64,
    #[serde(default)]
    pub next_rev: u64,
    #[serde(default)]
    pub next_audio_rev: u64,
    pub next_marker_id: u64,
    pub chunks: Vec<ChunkLocation>,
    pub current: SnapshotRecord,
    pub undo: Vec<EntryRecord>,
    pub redo: Vec<EntryRecord>,
}

impl CheckpointRecord {
    /// A checkpoint of `history` with the given chunk index.
    pub fn from_history(history: &History, chunks: Vec<ChunkLocation>) -> Self {
        CheckpointRecord {
            seq: history.current_seq(),
            saved_seq: history.saved_seq(),
            next_seq: history.next_seq(),
            next_rev: history.next_rev(),
            next_audio_rev: history.next_audio_rev(),
            next_marker_id: history.next_marker_id(),
            chunks,
            current: SnapshotRecord::from_snapshot(history.current()),
            undo: history
                .undo_entries()
                .iter()
                .map(EntryRecord::from_entry)
                .collect(),
            redo: history
                .redo_entries()
                .iter()
                .map(EntryRecord::from_entry)
                .collect(),
        }
    }

    /// The history this checkpoint describes (recovery, ADR-004 §9 step 4).
    pub(crate) fn to_history(&self) -> History {
        History::from_parts(
            self.current.to_snapshot(),
            self.undo.iter().map(EntryRecord::to_entry).collect(),
            self.redo.iter().map(EntryRecord::to_entry).collect(),
            self.next_rev,
            self.next_audio_rev,
            self.next_seq,
            self.saved_seq,
            self.next_marker_id,
        )
    }
}

/// A journal record (ADR-004 §6). `take_discard` is an addition of T-101: it closes a
/// `take_begin` that produced nothing to commit.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Record {
    Open {
        format_version: u32,
        sample_rate_hz: u32,
        #[serde(default)]
        source: Option<SourceRecord>,
    },
    Chunks {
        chunks: Vec<ChunkLocation>,
    },
    Edit(EditRecord),
    Undo {
        seq: u64,
    },
    Redo {
        seq: u64,
    },
    /// ADR-004 Amendment 3 (SPEC-004 §2.5 OD-1 = A): the `count` oldest undo entries were
    /// removed under disk pressure.
    DropUndo {
        count: u64,
    },
    TakeBegin {
        take: u32,
        mode: TakeModeRecord,
        at: u64,
        /// T-304 (SPEC-022 §4.6): `E − S` for a punch, else 0.
        len: u64,
        /// Session-relative path of the first take file.
        file: String,
        sample_rate_hz: u32,
        /// T-304 (SPEC-022 §4.6, ADR-004 Amendment 4): the punch/overwrite crossfade length.
        #[serde(default, skip_serializing_if = "is_zero_u64")]
        xfade_samples: u64,
        /// T-304: the recording offset δ applied to the aligned start (ns, signed).
        #[serde(default, skip_serializing_if = "is_zero_i64")]
        offset_ns: i64,
        /// T-304: the record window opens at an aligned start (it then needs a `take_window`
        /// record before anything of it is applicable).
        #[serde(default, skip_serializing_if = "is_false")]
        aligned: bool,
    },
    TakeDiscard {
        take: u32,
    },
    /// T-304 (SPEC-022 §4.6): the record window of an aligned take opened at take sample
    /// `k_start` (recovery reproduces the live alignment from it).
    TakeWindow {
        take: u32,
        k_start: u64,
    },
    /// T-304 (SPEC-022 §2.10, §4.6): a record operation was cancelled (Stop in pre-roll, device
    /// loss before the window opened): no edit, the take is not recoverable.
    TakeCancel {
        take: u32,
    },
    State {
        state: serde_json::Value,
    },
    Saved {
        path: String,
        seq: u64,
        format: String,
        /// T-306 (SPEC-018 §4.5): the file fingerprint (`document.audio_crc32`) as of this save,
        /// additive so an older journal (no sidecar yet) still parses.
        #[serde(default)]
        audio_crc32: Option<String>,
        /// `false` when the sidecar write failed (SPEC-018 §2.9); `None` on an older journal, or
        /// a save with no sidecar concept (never produced by this build, kept for forward compat).
        #[serde(default)]
        sidecar: Option<bool>,
        /// T-301 (ADR-004 Amendment 3): the written file's size and mtime, so recovery can tell
        /// "changed on disk since" from our own save (SPEC-004 §2.7).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        file_size_bytes: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        file_mtime_unix_ms: Option<u64>,
    },
    Checkpoint(CheckpointRecord),
    Close,
}

fn is_zero_u64(v: &u64) -> bool {
    *v == 0
}

fn is_zero_i64(v: &i64) -> bool {
    *v == 0
}

fn is_false(v: &bool) -> bool {
    !*v
}

/// Encodes one record as a journal line (`<crc32 hex>\t<json>\n`).
pub fn encode_record(record: &Record) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    encode_record_into(record, &mut out)?;
    Ok(out)
}

fn encode_record_into(record: &Record, out: &mut Vec<u8>) -> Result<()> {
    let json = serde_json::to_vec(record)?;
    let crc = crc32fast::hash(&json);
    // Writing into a Vec can't fail.
    let _ = write!(out, "{crc:08x}\t");
    out.extend_from_slice(&json);
    out.push(b'\n');
    Ok(())
}

/// The valid prefix of a journal.
#[derive(Clone, Debug, PartialEq)]
pub struct JournalContents {
    pub records: Vec<Record>,
    /// Length of the valid prefix in bytes.
    pub valid_bytes: u64,
    /// `true` if bytes follow the valid prefix (a torn line, bad CRC or unparseable record).
    pub damaged: bool,
}

/// Parses journal bytes, stopping at the first torn line, bad CRC or unparseable record. Never
/// panics on any input.
pub fn parse_journal(bytes: &[u8]) -> JournalContents {
    let mut records = Vec::new();
    let mut pos = 0usize;
    while pos < bytes.len() {
        let Some(nl) = bytes[pos..].iter().position(|&b| b == b'\n') else {
            break;
        };
        let Some(record) = parse_line(&bytes[pos..pos + nl]) else {
            break;
        };
        records.push(record);
        pos += nl + 1;
    }
    JournalContents {
        records,
        valid_bytes: pos as u64,
        damaged: pos < bytes.len(),
    }
}

fn parse_line(line: &[u8]) -> Option<Record> {
    let (crc_hex, rest) = (line.get(..8)?, line.get(8..)?);
    let json = rest.strip_prefix(b"\t")?;
    // Exactly the lowercase form the writer emits: a case flip in the CRC is damage too.
    if !crc_hex
        .iter()
        .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(c))
    {
        return None;
    }
    let crc = u32::from_str_radix(std::str::from_utf8(crc_hex).ok()?, 16).ok()?;
    if crc32fast::hash(json) != crc {
        return None;
    }
    serde_json::from_slice(json).ok()
}

/// [`parse_journal`] plus what recovery needs: the byte offset just after each record, and how
/// many lines follow the valid prefix.
#[derive(Clone, Debug, PartialEq)]
pub struct JournalScan {
    pub records: Vec<Record>,
    /// `ends[i]`: byte offset just past record `i`'s newline.
    pub ends: Vec<u64>,
    /// Length of the valid prefix in bytes.
    pub valid_bytes: u64,
    /// Lines (complete or torn) after the valid prefix.
    pub damaged_lines: usize,
    /// Of those, lines that are recognisably a `chunks` record (not a change by themselves).
    pub damaged_chunk_lines: usize,
    /// The journal's last line (valid or damaged) is a `chunks` record: the edit written in the
    /// same append never made it.
    pub ends_with_chunks: bool,
}

impl JournalScan {
    /// Changes lost after the valid prefix. A `chunks` record is always appended together with
    /// the edit that references it, so a damaged one isn't a change by itself — but a trailing
    /// one means its edit is gone.
    pub fn damaged_changes(&self) -> usize {
        self.damaged_lines - self.damaged_chunk_lines + usize::from(self.ends_with_chunks)
    }
}

/// Like [`parse_journal`], with record end offsets and a count of the damaged lines. Never
/// panics on any input.
pub fn scan_journal(bytes: &[u8]) -> JournalScan {
    let mut records = Vec::new();
    let mut ends = Vec::new();
    let mut pos = 0usize;
    while pos < bytes.len() {
        let Some(nl) = bytes[pos..].iter().position(|&b| b == b'\n') else {
            break;
        };
        let Some(record) = parse_line(&bytes[pos..pos + nl]) else {
            break;
        };
        records.push(record);
        pos += nl + 1;
        ends.push(pos as u64);
    }
    let tail = &bytes[pos..];
    let lines = tail.split(|&b| b == b'\n').filter(|l| !l.is_empty());
    let (mut damaged_lines, mut damaged_chunk_lines) = (0, 0);
    let mut last_is_chunks = matches!(records.last(), Some(Record::Chunks { .. }));
    for line in lines {
        damaged_lines += 1;
        last_is_chunks = line
            .windows(CHUNKS_TAG.len())
            .any(|w| w == CHUNKS_TAG.as_bytes());
        if last_is_chunks {
            damaged_chunk_lines += 1;
        }
    }
    JournalScan {
        records,
        ends,
        valid_bytes: pos as u64,
        damaged_lines,
        damaged_chunk_lines,
        ends_with_chunks: last_is_chunks,
    }
}

const CHUNKS_TAG: &str = "\"type\":\"chunks\"";

/// Reads and parses the journal at `path`.
pub fn read_journal(path: &Path) -> Result<JournalContents> {
    let bytes = std::fs::read(path).map_err(ProjectError::io("reading the journal"))?;
    Ok(parse_journal(&bytes))
}

/// An open journal file.
#[derive(Debug)]
pub struct Journal {
    path: PathBuf,
    file: File,
    len: u64,
}

impl Journal {
    /// Creates a new, empty journal (the file must not exist).
    pub fn create(path: &Path) -> Result<Journal> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(path)
            .map_err(ProjectError::io("creating the journal"))?;
        if let Some(parent) = path.parent() {
            sync_dir(parent).map_err(ProjectError::io("creating the journal"))?;
        }
        Ok(Journal {
            path: path.to_path_buf(),
            file,
            len: 0,
        })
    }

    /// Opens an existing journal for appending. A torn or corrupt tail (bytes after the valid
    /// prefix, [`parse_journal`]) is truncated away first and the truncation `fdatasync`ed, so
    /// new records are never hidden behind a bad line.
    pub fn open_append(path: &Path) -> Result<Journal> {
        const CONTEXT: &str = "opening the journal";
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .map_err(ProjectError::io(CONTEXT))?;
        let bytes = std::fs::read(path).map_err(ProjectError::io(CONTEXT))?;
        let contents = parse_journal(&bytes);
        if contents.damaged {
            file.set_len(contents.valid_bytes)
                .and_then(|()| file.sync_data())
                .map_err(ProjectError::io(CONTEXT))?;
        }
        Ok(Journal {
            path: path.to_path_buf(),
            file,
            len: contents.valid_bytes,
        })
    }

    /// Opens an existing journal for appending after cutting it to its first `len` bytes (and
    /// `fdatasync`ing the cut): recovery drops everything after the last record it could use
    /// (ADR-004 §9 step 4.2).
    pub fn open_truncated(path: &Path, len: u64) -> Result<Journal> {
        const CONTEXT: &str = "opening the journal";
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .map_err(ProjectError::io(CONTEXT))?;
        let current = file.metadata().map_err(ProjectError::io(CONTEXT))?.len();
        if current != len {
            file.set_len(len.min(current))
                .and_then(|()| file.sync_data())
                .map_err(ProjectError::io(CONTEXT))?;
        }
        Ok(Journal {
            path: path.to_path_buf(),
            file,
            len: len.min(current),
        })
    }

    /// Path of the journal file.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Current length in bytes.
    pub fn len_bytes(&self) -> u64 {
        self.len
    }

    /// Appends `records` with one write and `fdatasync`s (ADR-004 §6 durability table). On
    /// failure the file is truncated back, so a torn tail never hides later appends.
    pub fn append(&mut self, records: &[Record]) -> Result<()> {
        let mut buf = Vec::new();
        for record in records {
            encode_record_into(record, &mut buf)?;
        }
        if let Err(e) =
            write_all_at(&self.file, &buf, self.len).and_then(|()| self.file.sync_data())
        {
            let _ = self.file.set_len(self.len);
            return Err(ProjectError::from_io("appending to the journal", e));
        }
        self.len += buf.len() as u64;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_records() -> Vec<Record> {
        let edit = Edit::new("history.record")
            .with_label_param("target", "−1")
            .replace(0, 0, vec![Piece::chunk(0, 0, 65_536), Piece::silence(480)])
            .replace_with(10, 5, vec![Piece::silence(5)], MarkerMapping::Identity)
            .marker(MarkerOp::Add(Marker::new(
                MarkerId(1),
                10,
                0,
                "Dropout 10 ms",
            )))
            .with_attachment(vec![1, 2, 3, 0xff]);
        vec![
            Record::Open {
                format_version: 1,
                sample_rate_hz: 48_000,
                source: None,
            },
            Record::Chunks {
                chunks: vec![ChunkLocation {
                    id: 0,
                    offset: 0,
                    len: 65_536,
                    crc32: 0xdead_beef,
                }],
            },
            Record::TakeBegin {
                take: 1,
                mode: TakeModeRecord::New,
                at: 0,
                len: 0,
                file: "takes/take-0001.wav".into(),
                sample_rate_hz: 48_000,
                xfade_samples: 0,
                offset_ns: 0,
                aligned: false,
            },
            Record::Edit(EditRecord::from_edit(1, &edit, Some(1))),
            Record::Undo { seq: 1 },
            Record::Redo { seq: 1 },
            Record::State {
                state: serde_json::json!({"rack": [1.5, "x"]}),
            },
            Record::Saved {
                path: "/a/b.wav".into(),
                seq: 1,
                format: "wav24".into(),
                audio_crc32: Some("9f3a51c2".into()),
                sidecar: Some(true),
                file_size_bytes: Some(1234),
                file_mtime_unix_ms: Some(5678),
            },
            Record::DropUndo { count: 3 },
            Record::Close,
        ]
    }

    #[test]
    fn records_round_trip() {
        let records = sample_records();
        let mut bytes = Vec::new();
        for r in &records {
            bytes.extend(encode_record(r).unwrap());
        }
        let parsed = parse_journal(&bytes);
        assert_eq!(parsed.records, records);
        assert!(!parsed.damaged);
        assert_eq!(parsed.valid_bytes, bytes.len() as u64);
        let Record::Edit(e) = &parsed.records[3] else {
            panic!("not an edit")
        };
        let edit = e.to_edit();
        assert_eq!(edit.attachment, Some(vec![1, 2, 3, 0xff]));
        // ADR-004 Amendment 3: label params round-trip ("Normalize to −1 dB").
        assert_eq!(
            edit.label_params.get("target").map(String::as_str),
            Some("−1")
        );
        assert!(matches!(
            edit.ops[1],
            EditOp::Replace {
                mapping: MarkerMapping::Identity,
                ..
            }
        ));
        assert_eq!(EditRecord::from_edit(1, &edit, Some(1)), *e);
        let line = String::from_utf8(encode_record(&Record::Close).unwrap()).unwrap();
        assert!(line.ends_with("\t{\"type\":\"close\"}\n"), "{line}");
        let edit_line = String::from_utf8(encode_record(&records[3]).unwrap()).unwrap();
        assert!(
            edit_line.contains("\"mapping\":\"identity\""),
            "{edit_line}"
        );
    }

    /// SPEC-004 §4 fault injection (parser part): truncation at every byte offset inside the last
    /// three records, and one flipped byte per record, never panic and stop at the first bad line.
    #[test]
    fn parser_stops_at_first_damage() {
        let records = sample_records();
        let lines: Vec<Vec<u8>> = records.iter().map(|r| encode_record(r).unwrap()).collect();
        let bytes: Vec<u8> = lines.concat();
        let ends: Vec<usize> = lines
            .iter()
            .scan(0, |acc, l| {
                *acc += l.len();
                Some(*acc)
            })
            .collect();
        let tail_start = ends[ends.len() - 4];
        for cut in tail_start..=bytes.len() {
            let parsed = parse_journal(&bytes[..cut]);
            let complete = ends.iter().filter(|&&e| e <= cut).count();
            assert_eq!(parsed.records.len(), complete, "cut at {cut}");
            assert_eq!(parsed.records[..], records[..complete]);
            assert_eq!(parsed.damaged, cut != ends[complete.max(1) - 1] && cut != 0);
        }
        for (i, _) in lines.iter().enumerate() {
            let start = if i == 0 { 0 } else { ends[i - 1] };
            for offset in [start, start + 3, start + 9, ends[i] - 2] {
                let mut damaged = bytes.clone();
                damaged[offset] ^= 0x20;
                let parsed = parse_journal(&damaged);
                assert_eq!(
                    parsed.records[..],
                    records[..i],
                    "record {i} offset {offset}"
                );
                assert!(parsed.damaged);
            }
        }
    }

    #[test]
    fn open_append_truncates_a_torn_tail() {
        let path = std::env::temp_dir().join(format!("vox-project-journal-{}", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let mut journal = Journal::create(&path).unwrap();
        journal.append(&[Record::Undo { seq: 1 }]).unwrap();
        let valid = journal.len_bytes();
        drop(journal);
        // A torn half-line after the valid prefix.
        let mut bytes = std::fs::read(&path).unwrap();
        bytes.extend_from_slice(b"0badc0de\t{\"type\":\"re");
        std::fs::write(&path, &bytes).unwrap();

        let mut journal = Journal::open_append(&path).unwrap();
        assert_eq!(journal.len_bytes(), valid);
        assert_eq!(std::fs::metadata(&path).unwrap().len(), valid);
        journal.append(&[Record::Redo { seq: 1 }]).unwrap();
        let parsed = read_journal(&path).unwrap();
        assert!(!parsed.damaged);
        assert_eq!(
            parsed.records,
            vec![Record::Undo { seq: 1 }, Record::Redo { seq: 1 }]
        );
        std::fs::remove_file(path).unwrap();
    }
}

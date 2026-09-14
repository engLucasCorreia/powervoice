//! The session directory and the control-thread API over store + journal + history
//! (ADR-004 §1, §6, §7).

use std::fmt;
use std::fs::{self, File, OpenOptions, TryLockError};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

use crate::fs_util::{sync_dir, unix_ms, write_all_at, write_file_atomic};
use crate::history::{
    Edit, EditOp, History, HistoryStep, MarkerMapping, MarkerOp, check_marker_bounds,
};
use crate::journal::{
    CheckpointRecord, EditRecord, Journal, MarkerRecord, Record, SourceRecord, TakeModeRecord,
    journal_file_name,
};
use crate::snapshot::{DocSnapshot, Marker, MarkerId, Piece, Source};
use crate::store::{ChunkId, ChunkLocation, ChunkStore, ChunkWriter, StoreOptions, WrittenAudio};
use crate::take::{TakeSyncHandle, TakeWriter, TakeWriterOptions, take_part_path};
use crate::{CHUNK_SAMPLES, ProjectError, Result, SEGMENT_BYTES, STORE_FORMAT_VERSION};

/// Undo label key of a recording take (SPEC-002 §2.2, AC-5).
pub const TAKE_LABEL_KEY: &str = "history.record";
/// Advisory lock file (PID, host, start time).
pub const LOCK_FILE_NAME: &str = "session.lock";
/// Session metadata.
pub const META_FILE_NAME: &str = "meta.json";
/// Directory of take WAVs.
pub const TAKES_DIR_NAME: &str = "takes";

/// The user file a document was imported from (journal `open`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceInfo {
    pub path: PathBuf,
    pub size_bytes: u64,
    pub mtime_unix_ms: u64,
    pub format: String,
}

/// How to create a session.
#[derive(Clone, Debug)]
pub struct SessionConfig {
    /// Document sample rate.
    pub sample_rate_hz: u32,
    /// The imported file, or `None` for a new recording.
    pub source: Option<SourceInfo>,
    /// Chunk-store options (memory budget).
    pub store: StoreOptions,
}

impl SessionConfig {
    /// A new, empty document at `sample_rate_hz` with default store options.
    pub fn new(sample_rate_hz: u32) -> Self {
        SessionConfig {
            sample_rate_hz,
            source: None,
            store: StoreOptions::default(),
        }
    }
}

/// `meta.json` (ADR-004 §1).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Meta {
    pub format_version: u32,
    pub generation: u32,
    pub sample_rate_hz: u32,
    #[serde(default)]
    pub source_path: Option<String>,
    pub created_unix_ms: u64,
    pub chunk_samples: u32,
    pub segment_bytes: u64,
}

pub(crate) fn read_meta(dir: &Path) -> Option<Meta> {
    let bytes = fs::read(dir.join(META_FILE_NAME)).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// Identifies a take within a session (`takes/take-NNNN.wav`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TakeId(pub u32);

/// Where a take lands when it is committed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TakeMode {
    /// New-file recording (M1): appended at the end of the (normally empty) document.
    New,
    /// Inserted at a document position (record at cursor, M3).
    Insert { at_samples: u64 },
    /// T-304 (SPEC-022 §2.5): overwrites from a document position, extending the document when
    /// the take runs past its end (an open-ended punch).
    Overwrite { at_samples: u64 },
    /// T-304 (SPEC-022 §2.6): replaces exactly `[start_samples, end_samples)` (a punch-in).
    Punch {
        start_samples: u64,
        end_samples: u64,
    },
}

/// Undo label key of a punch-in (SPEC-022 §2.11).
pub const PUNCH_LABEL_KEY: &str = "history.punch";
/// Undo label key of Add Marker (SPEC-009 §2.2) — also what each marker added during a cancelled
/// record operation becomes (SPEC-022 §2.9, H-21).
pub const MARKER_ADD_LABEL_KEY: &str = "history.marker_add";

/// T-304 (SPEC-022 §4.6): how an operation's record window is aligned and joined.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TakeParams {
    /// Punch/Overwrite crossfade length `X` (SPEC-022 §2.8), in document samples.
    pub xfade_samples: u64,
    /// The recording offset δ applied to the aligned start (journaled, informational).
    pub offset_ns: i64,
    /// The window opens at an aligned start (the engine reports `k_start` later through
    /// [`Session::note_take_window`]); otherwise it is a free start (`k_start = 0`).
    pub aligned: bool,
}

/// T-304: everything the journal knows about an open take (`take_begin` + `take_window`), so a
/// live commit and crash recovery build the same edit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TakePlan {
    pub mode: TakeModeRecord,
    /// Insertion point / record point `at` (the punch start `S`).
    pub at: u64,
    /// Punch: `E − S`; otherwise 0.
    pub len: u64,
    pub xfade_samples: u64,
    pub offset_ns: i64,
    pub aligned: bool,
    /// Take sample the record window starts at (`None`: an aligned window not opened yet).
    pub k_start: Option<u64>,
}

/// The capture side of an open take, owned by the engine's capture-writer thread: every block
/// goes to the crash-safe WAV first (authoritative), then to the chunk store.
pub struct TakeCapture {
    id: TakeId,
    wav: TakeWriter,
    chunks: ChunkWriter,
}

impl TakeCapture {
    /// The take's id.
    pub fn id(&self) -> TakeId {
        self.id
    }

    /// Appends captured samples (call at least every 50 ms, SPEC-002 §4.1). With the default
    /// [`crate::take::TakeSyncMode::Background`] it never waits for the disk: run
    /// [`Self::sync_handle`]'s `sync()` about every second on another thread.
    pub fn append(&mut self, samples: &[f32]) -> Result<()> {
        self.wav.append(samples)?;
        self.chunks.append(samples)
    }

    /// A handle that patches the WAV header + `fdatasync`s from another thread.
    pub fn sync_handle(&self) -> TakeSyncHandle {
        self.wav.sync_handle()
    }

    /// Patches the WAV header and `fdatasync`s now, on this thread.
    pub fn sync(&mut self) -> Result<()> {
        self.wav.sync()
    }

    /// Samples captured so far.
    pub fn samples_written(&self) -> u64 {
        self.wav.samples_written()
    }

    /// The WAV part files written so far.
    pub fn wav_parts(&self) -> &[PathBuf] {
        self.wav.parts()
    }

    /// Commits the final chunk, patches + `fdatasync`s the WAV (ADR-004 §7 step 3). Never loses
    /// what is already durable: on failure `audio` is what the store holds ("finalize at the last
    /// good sample") and `error` says what went wrong.
    pub fn finish(self) -> FinishedTake {
        let (audio, chunk_error) = self.chunks.finish_lossy();
        let wav_parts = self.wav.parts().to_vec();
        let wav_samples = self.wav.samples_written();
        let wav_error = self.wav.finish().err();
        FinishedTake {
            take: self.id,
            audio,
            wav_parts,
            wav_samples,
            error: wav_error.or(chunk_error),
        }
    }
}

/// A take whose capture has stopped, ready for [`Session::commit_take`].
#[derive(Debug)]
pub struct FinishedTake {
    pub take: TakeId,
    /// The take's audio in the chunk store.
    pub audio: WrittenAudio,
    pub wav_parts: Vec<PathBuf>,
    pub wav_samples: u64,
    /// First failure while finishing, if any (the engine posts a notice).
    pub error: Option<ProjectError>,
}

/// Why [`Session::close`] failed. `session` is `Some` when nothing was closed and the caller can
/// retry or keep using it (recording, store still shared, journal append failed); it is `None`
/// when `close` was journaled but deleting the directory failed — the next start-up finishes the
/// deletion (the journal ends in `close`).
#[derive(Debug, thiserror::Error)]
#[error("closing the session failed: {error}")]
pub struct CloseError {
    pub error: ProjectError,
    pub session: Option<Box<Session>>,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct OpenTake {
    pub(crate) id: TakeId,
    pub(crate) plan: TakePlan,
}

/// Which way a boundary crossfade runs (SPEC-022 §4.2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FadeDir {
    /// Punch-in: from the old audio `A` to the take `T`.
    In,
    /// Punch-out: from the take `T` back to `A`.
    Out,
}

/// One open document's session: directory + lock + store + journal + history. Owned by the
/// engine's control thread. Every document-changing command appends and `fdatasync`s its journal
/// record before returning `Ok` (ADR-004 §6); on error nothing changed.
pub struct Session {
    pub(crate) id: String,
    pub(crate) dir: PathBuf,
    pub(crate) _lock: File,
    pub(crate) store: Arc<ChunkStore>,
    pub(crate) journal: Journal,
    pub(crate) history: History,
    pub(crate) sample_rate_hz: u32,
    pub(crate) journaled_chunks: Vec<bool>,
    pub(crate) open_take: Option<OpenTake>,
    /// H-17 (SPEC-004 §2.7): markers pressed during the open take, journaled as they happen
    /// (`Self::note_take_marker`) — the crash-recovery source [`Self::apply_open_take_from_wav`]
    /// applies. Live commits (`Self::commit_take`/`Self::commit_take_window`) still take their
    /// markers as an argument, independent of this. Cleared whenever the take closes.
    pub(crate) open_take_markers: Vec<Marker>,
    pub(crate) next_take: u32,
    /// Current store generation (`chunks.<gen>.f32`, `journal.<gen>.jsonl`; T-301 compaction).
    pub(crate) generation: u32,
    /// Options new stores of this session are created with (compaction).
    pub(crate) store_options: StoreOptions,
    pub(crate) meta: Meta,
    /// The journal's `open` record, the latest `saved` and `state` records: compaction copies
    /// them into the next generation's journal (T-301).
    pub(crate) open_record: Record,
    pub(crate) last_saved: Option<Record>,
    pub(crate) last_state: Option<Record>,
}

impl fmt::Debug for Session {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Session")
            .field("id", &self.id)
            .field("dir", &self.dir)
            .field("rev", &self.history.current().rev)
            .field("recording", &self.open_take.is_some())
            .finish_non_exhaustive()
    }
}

impl Session {
    /// Creates `<sessions_dir>/<id>/`, locks it, writes `meta.json` and starts the journal with
    /// `open` + a checkpoint of the empty undo floor.
    pub fn create(sessions_dir: &Path, config: SessionConfig) -> Result<Session> {
        if config.sample_rate_hz == 0 {
            return Err(ProjectError::InvalidArgument("sample rate must be > 0"));
        }
        fs::create_dir_all(sessions_dir)
            .map_err(ProjectError::io("creating the sessions directory"))?;
        let (id, dir) = create_unique_dir(sessions_dir)?;
        sync_dir(sessions_dir)
            .map_err(ProjectError::io("creating the session directory"))
            .and_then(|()| Self::init(id, dir.clone(), config))
            .inspect_err(|_| {
                let _ = fs::remove_dir_all(&dir);
            })
    }

    fn init(id: String, dir: PathBuf, config: SessionConfig) -> Result<Session> {
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(dir.join(LOCK_FILE_NAME))
            .map_err(ProjectError::io("creating the session lock"))?;
        match lock.try_lock() {
            Ok(()) => {}
            Err(TryLockError::WouldBlock) => return Err(ProjectError::SessionLocked),
            Err(TryLockError::Error(e)) => {
                return Err(ProjectError::from_io("locking the session", e));
            }
        }
        let now = unix_ms(SystemTime::now());
        write_lock_info(&lock)?;

        let meta = Meta {
            format_version: STORE_FORMAT_VERSION,
            generation: 0,
            sample_rate_hz: config.sample_rate_hz,
            source_path: config
                .source
                .as_ref()
                .map(|s| s.path.to_string_lossy().into_owned()),
            created_unix_ms: now,
            chunk_samples: CHUNK_SAMPLES as u32,
            segment_bytes: SEGMENT_BYTES,
        };
        write_file_atomic(
            &dir.join(META_FILE_NAME),
            &serde_json::to_vec_pretty(&meta)?,
        )
        .map_err(ProjectError::io("writing meta.json"))?;
        fs::create_dir(dir.join(TAKES_DIR_NAME))
            .map_err(ProjectError::io("creating the takes directory"))?;

        let store_options = config.store.clone();
        let store = ChunkStore::create(&dir, 0, config.store)?;
        let mut journal = Journal::create(&dir.join(journal_file_name(0)))?;
        let history = History::new(DocSnapshot::empty(config.sample_rate_hz));
        let open_record = Record::Open {
            format_version: STORE_FORMAT_VERSION,
            sample_rate_hz: config.sample_rate_hz,
            source: config.source.as_ref().map(|s| SourceRecord {
                path: s.path.to_string_lossy().into_owned(),
                size_bytes: s.size_bytes,
                mtime_unix_ms: s.mtime_unix_ms,
                format: s.format.clone(),
            }),
        };
        journal.append(&[
            open_record.clone(),
            Record::Checkpoint(CheckpointRecord::from_history(&history, Vec::new())),
        ])?;
        sync_dir(&dir).map_err(ProjectError::io("creating the session"))?;
        Ok(Session {
            id,
            dir,
            _lock: lock,
            store,
            journal,
            history,
            sample_rate_hz: config.sample_rate_hz,
            journaled_chunks: Vec::new(),
            open_take: None,
            open_take_markers: Vec::new(),
            next_take: 1,
            generation: 0,
            store_options,
            meta,
            open_record,
            last_saved: None,
            last_state: None,
        })
    }

    /// The session id (directory name).
    pub fn id(&self) -> &str {
        &self.id
    }

    /// The session directory.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// The takes directory.
    pub fn takes_dir(&self) -> PathBuf {
        self.dir.join(TAKES_DIR_NAME)
    }

    /// Path of the current journal.
    pub fn journal_path(&self) -> &Path {
        self.journal.path()
    }

    /// The chunk store, to share with reader and writer threads.
    pub fn store(&self) -> &Arc<ChunkStore> {
        &self.store
    }

    /// Document sample rate.
    pub fn sample_rate_hz(&self) -> u32 {
        self.sample_rate_hz
    }

    /// The current snapshot.
    pub fn current(&self) -> Arc<DocSnapshot> {
        Arc::clone(self.history.current())
    }

    /// Undo/redo state (depths, labels, dirty).
    pub fn history(&self) -> &History {
        &self.history
    }

    /// `true` while the document differs from its last saved state.
    pub fn is_dirty(&self) -> bool {
        self.history.is_dirty()
    }

    /// `true` while a take is open.
    pub fn is_recording(&self) -> bool {
        self.open_take.is_some()
    }

    /// The open take, if any.
    pub fn open_take(&self) -> Option<TakeId> {
        self.open_take.map(|t| t.id)
    }

    /// The open take and where it will be inserted.
    pub fn open_take_at(&self) -> Option<(TakeId, u64)> {
        self.open_take.map(|t| (t.id, t.plan.at))
    }

    /// H-21: the markers added during the open take so far ([`Self::note_take_marker`], document
    /// time), which its commit will contain.
    pub fn open_take_markers(&self) -> &[Marker] {
        &self.open_take_markers
    }

    /// The current store generation (T-301 compaction bumps it).
    pub fn generation(&self) -> u32 {
        self.generation
    }

    /// A fresh marker id.
    pub fn new_marker_id(&mut self) -> MarkerId {
        self.history.allocate_marker_id()
    }

    /// A [`ChunkWriter`] for new audio (edit ops on a worker thread).
    pub fn chunk_writer(&self) -> ChunkWriter {
        self.store.writer()
    }

    /// Makes `audio` (and `markers`, e.g. imported cue markers) the **undo floor**: the imported
    /// file (SPEC-004 §2.1, ADR-004 §8). This is not an undoable edit; the document stays clean
    /// (saved seq 0). Allowed only before the first edit or take. Syncs the chunks, then
    /// journals `chunks` + a `checkpoint` of the new floor.
    pub fn set_floor(
        &mut self,
        audio: &WrittenAudio,
        markers: Vec<Marker>,
    ) -> Result<Arc<DocSnapshot>> {
        if self.open_take.is_some() {
            return Err(ProjectError::NotWhileRecording);
        }
        if self.history.undo_depth() > 0 || self.history.redo_depth() > 0 {
            return Err(ProjectError::InvalidEdit(
                "the undo floor can only be set before the first edit".into(),
            ));
        }
        self.validate_piece_refs(&audio.pieces)?;
        let floor = DocSnapshot::new(self.sample_rate_hz, audio.pieces.clone(), markers);
        for m in floor.markers.iter() {
            check_marker_bounds(m.pos_samples, m.len_samples, floor.len_samples)?;
        }
        let history = self.history.rebased(floor);
        let new_chunks = self.unjournaled_chunks(audio.pieces.iter());
        if !new_chunks.is_empty() {
            self.store.sync()?;
        }
        let mut index: Vec<ChunkLocation> = (0..self.journaled_chunks.len())
            .filter(|&i| self.journaled_chunks[i])
            .filter_map(|i| self.store.location(i as ChunkId))
            .chain(new_chunks.iter().copied())
            .collect();
        index.sort_unstable_by_key(|loc| loc.id);
        let mut records = Vec::with_capacity(2);
        if !new_chunks.is_empty() {
            records.push(Record::Chunks {
                chunks: new_chunks.clone(),
            });
        }
        records.push(Record::Checkpoint(CheckpointRecord::from_history(
            &history, index,
        )));
        self.journal.append(&records)?;
        for loc in &new_chunks {
            self.mark_journaled(loc.id);
        }
        self.history = history;
        Ok(self.current())
    }

    /// Commits an edit: syncs new chunks, journals `chunks` (for chunks not journaled yet) +
    /// `edit`, `fdatasync`s, then swaps the current snapshot. Audio edits are refused while
    /// recording ([`ProjectError::NotWhileRecording`], `error.not_while_recording`); marker-only
    /// edits are allowed.
    ///
    /// Markers the user adds **during a take** must not go through here: they are noted with
    /// [`Self::note_take_marker`] and committed with the take, so they belong to its single undo
    /// entry (SPEC-002 §2.2, SPEC-022 §2.9). `commit_edit` would record each as a separate entry.
    pub fn commit_edit(&mut self, edit: Edit) -> Result<HistoryStep> {
        if self.open_take.is_some() && edit.changes_audio() {
            return Err(ProjectError::NotWhileRecording);
        }
        self.commit_internal(&edit, None)
    }

    /// `take`: the committed take and, if its WAV holds more than the committed audio, the WAV
    /// length in samples.
    fn commit_internal(
        &mut self,
        edit: &Edit,
        take: Option<(TakeId, Option<u64>)>,
    ) -> Result<HistoryStep> {
        for op in &edit.ops {
            let EditOp::Replace { pieces, .. } = op;
            self.validate_piece_refs(pieces)?;
        }
        let prepared = self.history.prepare(edit)?;
        let new_chunks = self.unjournaled_chunks(edit.ops.iter().flat_map(|op| {
            let EditOp::Replace { pieces, .. } = op;
            pieces.iter()
        }));
        let mut records = Vec::with_capacity(2);
        if !new_chunks.is_empty() {
            // ADR-004 §2: chunk bytes are durable before any journal record references them.
            // This is the single place that enforces it.
            self.store.sync()?;
            records.push(Record::Chunks {
                chunks: new_chunks.clone(),
            });
        }
        let mut record = EditRecord::from_edit(prepared.seq(), edit, take.map(|(t, _)| t.0));
        record.take_wav_samples = take.and_then(|(_, wav_samples)| wav_samples);
        records.push(Record::Edit(record));
        self.journal.append(&records)?;
        for loc in &new_chunks {
            self.mark_journaled(loc.id);
        }
        Ok(self.history.commit(prepared))
    }

    fn validate_piece_refs(&self, pieces: &[Piece]) -> Result<()> {
        for piece in pieces {
            if let Source::Chunk(id) = piece.source {
                let loc = self
                    .store
                    .location(id)
                    .ok_or(ProjectError::UnknownChunk(id))?;
                if u64::from(piece.offset) + u64::from(piece.len) > u64::from(loc.len) {
                    return Err(ProjectError::OutOfBounds {
                        chunk: id,
                        offset: piece.offset,
                        len: piece.len,
                    });
                }
            }
        }
        Ok(())
    }

    fn unjournaled_chunks<'a>(
        &self,
        pieces: impl IntoIterator<Item = &'a Piece>,
    ) -> Vec<ChunkLocation> {
        let mut ids: Vec<ChunkId> = pieces
            .into_iter()
            .filter_map(|p| match p.source {
                Source::Chunk(id) if !self.is_journaled(id) => Some(id),
                _ => None,
            })
            .collect();
        ids.sort_unstable();
        ids.dedup();
        ids.into_iter()
            .filter_map(|id| self.store.location(id))
            .collect()
    }

    pub(crate) fn is_journaled(&self, id: ChunkId) -> bool {
        self.journaled_chunks
            .get(id as usize)
            .copied()
            .unwrap_or(false)
    }

    pub(crate) fn mark_journaled(&mut self, id: ChunkId) {
        let i = id as usize;
        if self.journaled_chunks.len() <= i {
            self.journaled_chunks.resize(i + 1, false);
        }
        self.journaled_chunks[i] = true;
    }

    /// Undoes the top entry (journal `undo` + `fdatasync` first). `Ok(None)` at the undo floor;
    /// refused while recording.
    pub fn undo(&mut self) -> Result<Option<HistoryStep>> {
        if self.open_take.is_some() {
            return Err(ProjectError::NotWhileRecording);
        }
        let Some(seq) = self.history.peek_undo_seq() else {
            return Ok(None);
        };
        self.journal.append(&[Record::Undo { seq }])?;
        Ok(self.history.undo())
    }

    /// Redoes the top entry (journal `redo` + `fdatasync` first). `Ok(None)` if nothing to redo;
    /// refused while recording.
    pub fn redo(&mut self) -> Result<Option<HistoryStep>> {
        if self.open_take.is_some() {
            return Err(ProjectError::NotWhileRecording);
        }
        let Some(seq) = self.history.peek_redo_seq() else {
            return Ok(None);
        };
        self.journal.append(&[Record::Redo { seq }])?;
        Ok(self.history.redo())
    }

    /// Opens a take (ADR-004 §7 step 1): journals `take_begin`, then creates
    /// `takes/take-NNNN.wav` (32-bit float at the document rate). From now on audio edits, undo
    /// and redo are refused until [`Self::commit_take`] or [`Self::discard_take`].
    pub fn begin_take(
        &mut self,
        mode: TakeMode,
        options: TakeWriterOptions,
    ) -> Result<TakeCapture> {
        self.begin_take_with(mode, TakeParams::default(), options)
    }

    /// The open take and its journaled plan (T-304).
    pub fn open_take_plan(&self) -> Option<(TakeId, TakePlan)> {
        self.open_take.map(|t| (t.id, t.plan))
    }

    /// T-304 (SPEC-022 §4.6): journals (`take_window` + `fdatasync`) that the aligned record
    /// window of `take` opened at take sample `k_start`, so recovery reproduces the live
    /// alignment. Idempotent for the same value.
    pub fn note_take_window(&mut self, take: TakeId, k_start: u64) -> Result<()> {
        let open = match self.open_take {
            Some(open) if open.id == take => open,
            _ => return Err(ProjectError::NoSuchTake(take.0)),
        };
        if open.plan.k_start == Some(k_start) {
            return Ok(());
        }
        self.journal.append(&[Record::TakeWindow {
            take: take.0,
            k_start,
        }])?;
        if let Some(open) = self.open_take.as_mut() {
            open.plan.k_start = Some(k_start);
        }
        Ok(())
    }

    /// A marker added during the open take (M during a take or a record operation, SPEC-002
    /// §2.2, SPEC-022 §2.9): journaled (`take_marker` + `fdatasync`, H-17) so
    /// [`Self::apply_open_take_from_wav`] restores it after a crash, and kept for the take's
    /// commit — [`Self::commit_take`] / [`Self::commit_take_window`] add it to the take's one
    /// edit (placed per mode and phase, H-21), [`Self::cancel_take`] turns it into its own "Add
    /// Marker" entry. `marker.pos_samples` is document time (the heard position, or `at + k` in
    /// the record window).
    pub fn note_take_marker(&mut self, take: TakeId, marker: Marker) -> Result<()> {
        match self.open_take {
            Some(open) if open.id == take => {}
            _ => return Err(ProjectError::NoSuchTake(take.0)),
        }
        self.journal.append(&[Record::TakeMarker {
            take: take.0,
            marker: MarkerRecord::from_marker(&marker),
        }])?;
        self.open_take_markers.push(marker);
        Ok(())
    }

    /// T-304 (SPEC-022 §2.10): closes a cancelled operation's take with `take_cancel` (the audio
    /// is untouched) and deletes its take files (best effort). H-21 (§2.9, AC-10): markers added
    /// during it are never lost — each is then committed as its own ordinary "Add Marker" entry
    /// ([`MARKER_ADD_LABEL_KEY`]), as if added during playback, clamped into the document.
    pub fn cancel_take(&mut self, take: TakeId) -> Result<()> {
        let markers = self.close_cancelled_take(take)?;
        let len = self.history.current().len_samples;
        for marker in markers {
            let pos = marker.pos_samples.min(len);
            let end = marker.end_samples().clamp(pos, len);
            let edit = Edit::new(MARKER_ADD_LABEL_KEY).marker(MarkerOp::Add(Marker {
                pos_samples: pos,
                len_samples: end - pos,
                ..marker
            }));
            self.commit_internal(&edit, None)?;
        }
        Ok(())
    }

    /// Journals `take_cancel` for the open take `take`, closes it and deletes its files; returns
    /// the markers added during it.
    fn close_cancelled_take(&mut self, take: TakeId) -> Result<Vec<Marker>> {
        match self.open_take {
            Some(open) if open.id == take => {
                self.journal
                    .append(&[Record::TakeCancel { take: take.0 }])?;
                self.open_take = None;
                let markers = std::mem::take(&mut self.open_take_markers);
                let takes_dir = self.takes_dir();
                for part in 0.. {
                    let path = take_part_path(&takes_dir, take.0, part);
                    if fs::remove_file(&path).is_err() {
                        break;
                    }
                }
                Ok(markers)
            }
            _ => Err(ProjectError::NoSuchTake(take.0)),
        }
    }

    /// H-21 (SPEC-022 §2.9): where a marker added during a take lands in the take's edit, `n`
    /// being the samples the window commits and `doc_len` the length before it:
    /// - before `at` (pre-roll: the heard position on the existing audio) it stays;
    /// - in the record range it is `at + k`, clamped into the committed audio `[at, at + n]` (a
    ///   partial window ends early; a key press extrapolated a touch past Stop);
    /// - a punch's post-roll markers (at or after `E`) stay on the existing audio.
    ///
    /// Everything is clamped into the resulting document.
    fn place_take_marker(plan: &TakePlan, doc_len: u64, n: u64, marker: &Marker) -> Marker {
        let at = plan.at;
        let new_len = match plan.mode {
            TakeModeRecord::New | TakeModeRecord::Insert => doc_len + n,
            TakeModeRecord::Overwrite => doc_len.max(at + n),
            TakeModeRecord::Punch => doc_len,
        };
        let q = marker.pos_samples;
        let post_roll = plan.mode == TakeModeRecord::Punch && q >= at + plan.len;
        let (pos, hi) = if q < at || post_roll {
            (q, new_len)
        } else {
            (q.min(at + n), at + n)
        };
        let hi = hi.min(new_len);
        let pos = pos.min(hi);
        let end = marker.end_samples().clamp(pos, hi);
        Marker {
            pos_samples: pos,
            len_samples: end - pos,
            ..marker.clone()
        }
    }

    /// The marker ops of a take's edit: `extra` (e.g. dropout markers) plus the markers added
    /// during the take, placed by [`Self::place_take_marker`].
    fn take_marker_ops(&self, plan: &TakePlan, n: u64, extra: &[Marker]) -> Vec<MarkerOp> {
        let doc_len = self.history.current().len_samples;
        extra
            .iter()
            .chain(&self.open_take_markers)
            .map(|m| MarkerOp::Add(Self::place_take_marker(plan, doc_len, n, m)))
            .collect()
    }

    /// [`Self::begin_take`] for the record operations of SPEC-022 (T-304): the mode (Insert,
    /// Overwrite, Punch) and how the window is aligned/joined go into `take_begin`.
    pub fn begin_take_with(
        &mut self,
        mode: TakeMode,
        params: TakeParams,
        options: TakeWriterOptions,
    ) -> Result<TakeCapture> {
        if self.open_take.is_some() {
            return Err(ProjectError::TakeAlreadyOpen);
        }
        let len = self.history.current().len_samples;
        let (at, mode_record, plan_len) = match mode {
            TakeMode::New => (len, TakeModeRecord::New, 0),
            TakeMode::Insert { at_samples } if at_samples <= len => {
                (at_samples, TakeModeRecord::Insert, 0)
            }
            TakeMode::Overwrite { at_samples } if at_samples <= len => {
                (at_samples, TakeModeRecord::Overwrite, 0)
            }
            TakeMode::Punch {
                start_samples,
                end_samples,
            } if start_samples < end_samples && end_samples <= len => (
                start_samples,
                TakeModeRecord::Punch,
                end_samples - start_samples,
            ),
            _ => {
                return Err(ProjectError::InvalidEdit(
                    "take position is outside the document".into(),
                ));
            }
        };
        let plan = TakePlan {
            mode: mode_record,
            at,
            len: plan_len,
            xfade_samples: params.xfade_samples,
            offset_ns: params.offset_ns,
            aligned: params.aligned,
            k_start: (!params.aligned).then_some(0),
        };
        let id = TakeId(self.next_take);
        let takes_dir = self.takes_dir();
        let file = take_part_path(Path::new(TAKES_DIR_NAME), id.0, 0);
        self.journal.append(&[Record::TakeBegin {
            take: id.0,
            mode: mode_record,
            at,
            len: plan_len,
            file: file.to_string_lossy().replace('\\', "/"),
            sample_rate_hz: self.sample_rate_hz,
            xfade_samples: params.xfade_samples,
            offset_ns: params.offset_ns,
            aligned: params.aligned,
        }])?;
        self.next_take += 1;
        let wav = match TakeWriter::create(&takes_dir, id.0, self.sample_rate_hz, options) {
            Ok(wav) => wav,
            Err(e) => {
                // Close the journaled take_begin so the session doesn't look like it holds one.
                let _ = self.journal.append(&[Record::TakeDiscard { take: id.0 }]);
                return Err(e);
            }
        };
        self.open_take = Some(OpenTake { id, plan });
        self.open_take_markers.clear();
        Ok(TakeCapture {
            id,
            wav,
            chunks: self.store.writer(),
        })
    }

    /// Commits a finished take as **one** undoable edit labelled [`TAKE_LABEL_KEY`], containing
    /// its audio and `markers` (user and dropout markers placed during the take, in document
    /// time) — ADR-004 §7 step 3, SPEC-002 §2.2.
    ///
    /// Markers are clamped into the take's range `[at, at + audio length]`: a marker pressed just
    /// before Stop (extrapolated ±10 ms, SPEC-003) or past audio lost to a store failure must not
    /// sink the take. A take whose WAV is empty and that has no markers is closed with
    /// `take_discard` and adds no entry (`Ok(None)`). A take is never discarded while its WAV holds
    /// audio: if the chunk store holds none of it, [`ProjectError::TakeNotInStore`] is returned
    /// and the take stays open — drop the session so start-up recovery applies the take from its
    /// WAV. If the store holds less than the WAV, the edit records the WAV length
    /// (`take_wav_samples`) and GC keeps the session recoverable. Arguments are borrowed, so on error the take stays open and the
    /// caller can retry.
    pub fn commit_take(
        &mut self,
        finished: &FinishedTake,
        markers: &[Marker],
    ) -> Result<Option<HistoryStep>> {
        let open = match self.open_take {
            Some(open) if open.id == finished.take => open,
            _ => return Err(ProjectError::NoSuchTake(finished.take.0)),
        };
        let len = finished.audio.len_samples;
        if finished.wav_samples == 0
            && len == 0
            && markers.is_empty()
            && self.open_take_markers.is_empty()
        {
            self.journal
                .append(&[Record::TakeDiscard { take: open.id.0 }])?;
            self.open_take = None;
            self.open_take_markers.clear();
            return Ok(None);
        }
        if len == 0 && finished.wav_samples > 0 {
            // The audio exists only in the WAV (the store failed before its first chunk). A
            // discard would let GC delete the WAV, so the take stays open.
            return Err(ProjectError::TakeNotInStore {
                take: open.id.0,
                wav_samples: finished.wav_samples,
            });
        }
        let mut edit = Edit::new(TAKE_LABEL_KEY);
        if len > 0 {
            edit = edit.replace(open.plan.at, 0, finished.audio.pieces.clone());
        }
        for op in self.take_marker_ops(&open.plan, len, markers) {
            edit = edit.marker(op);
        }
        // A WAV longer than the committed audio (store failure mid-take) is recorded, so GC keeps
        // the session and the WAV tail instead of deleting them after a save.
        let truncated = (finished.wav_samples > len).then_some(finished.wav_samples);
        let step = self.commit_internal(&edit, Some((open.id, truncated)))?;
        self.open_take = None;
        self.open_take_markers.clear();
        Ok(Some(step))
    }

    /// Closes the open take without adding an undo entry (journal `take_discard`). The WAV is no
    /// longer offered for recovery afterwards.
    pub fn discard_take(&mut self, take: TakeId) -> Result<()> {
        match self.open_take {
            Some(open) if open.id == take => {
                self.journal
                    .append(&[Record::TakeDiscard { take: take.0 }])?;
                self.open_take = None;
                self.open_take_markers.clear();
                Ok(())
            }
            _ => Err(ProjectError::NoSuchTake(take.0)),
        }
    }

    /// Records a successful save of the current state (journal `saved`).
    pub fn mark_saved(&mut self, path: &Path, format: &str) -> Result<()> {
        self.mark_saved_with_sidecar(path, format, None, None)
    }

    /// [`Self::mark_saved_with_sidecar`], also recording the written file's `(size, mtime)` so
    /// crash recovery can tell a later change on disk from this save (T-301, SPEC-004 §2.7).
    pub fn mark_saved_file(
        &mut self,
        path: &Path,
        format: &str,
        audio_crc32: Option<String>,
        sidecar: Option<bool>,
        file_facts: Option<(u64, u64)>,
    ) -> Result<()> {
        let record = Record::Saved {
            path: path.to_string_lossy().into_owned(),
            seq: self.history.current_seq(),
            format: format.to_owned(),
            audio_crc32,
            sidecar,
            file_size_bytes: file_facts.map(|f| f.0),
            file_mtime_unix_ms: file_facts.map(|f| f.1),
        };
        self.journal.append(std::slice::from_ref(&record))?;
        self.history.mark_saved();
        self.last_saved = Some(record);
        Ok(())
    }

    /// [`Self::mark_saved`], additionally recording the file fingerprint and whether the sidecar
    /// write succeeded (SPEC-018 §4.5).
    pub fn mark_saved_with_sidecar(
        &mut self,
        path: &Path,
        format: &str,
        audio_crc32: Option<String>,
        sidecar: Option<bool>,
    ) -> Result<()> {
        self.mark_saved_file(path, format, audio_crc32, sidecar, None)
    }

    /// Journals sidecar-level rack/view state (the caller debounces, ADR-004 §6).
    pub fn append_state(&mut self, state: serde_json::Value) -> Result<()> {
        let record = Record::State { state };
        self.journal.append(std::slice::from_ref(&record))?;
        self.last_state = Some(record);
        Ok(())
    }

    /// Crash recovery's "Apply as recorded" (SPEC-004 §2.7): commits the open take from its WAV
    /// (authoritative, ADR-004 §9 step 5) as one undoable "Record" edit at its insertion point,
    /// exactly as if Stop had been pressed. Markers pressed during the take and journaled via
    /// [`Self::note_take_marker`] are restored, clamped into the committed range exactly like
    /// [`Self::commit_take`]'s own `markers` argument (H-17: previously such markers were never
    /// journaled and were lost here). An empty WAV closes the take with `take_discard` (`Ok(None)`).
    pub fn apply_open_take_from_wav(&mut self) -> Result<Option<HistoryStep>> {
        let open = self.open_take.ok_or(ProjectError::NoSuchTake(0))?;
        let parts = crate::take::recover_take(&self.takes_dir(), open.id.0)?;
        let mut writer = self.store.writer();
        crate::take::copy_take_into(&parts, &mut writer)?;
        let audio = writer.finish()?;
        // T-304 (SPEC-022 §2.12): "Apply as recorded" applies exactly what Stop at the recovered
        // end would have: the record window from the journaled `k_start` to the last sample.
        let Some(k_start) = open.plan.k_start else {
            // A crash during pre-roll: nothing applies, and the document stays exactly as it was
            // before the operation (SPEC-022 AC-15) — its markers included.
            self.close_cancelled_take(open.id)?;
            return Ok(None);
        };
        let window = (k_start, u64::MAX);
        let Some(mut edit) = self.build_take_edit(&open.plan, &audio.pieces, window)? else {
            self.journal
                .append(&[Record::TakeDiscard { take: open.id.0 }])?;
            self.open_take = None;
            self.open_take_markers.clear();
            return Ok(None);
        };
        let take_len: u64 = audio.pieces.iter().map(Piece::len_samples).sum();
        let n = Self::window_len(&open.plan, take_len, window);
        for op in self.take_marker_ops(&open.plan, n, &[]) {
            edit = edit.marker(op);
        }
        let step = self.commit_internal(&edit, Some((open.id, None)))?;
        self.open_take = None;
        self.open_take_markers.clear();
        Ok(Some(step))
    }

    /// T-304 (SPEC-022 §2.4–§2.11, §4.1): commits the record window `take[window.0, window.1)`
    /// of a finished take as **one** undoable edit according to the open take's mode — Insert
    /// (spliced exactly, markers shift), Overwrite (replaces/extends, equal-power fades inside the
    /// new range, markers stay) or Punch (replaces exactly `[S, S + n)`, fades inside, markers
    /// stay) — with `markers` (document time; clamped into the new audio). The window is clipped
    /// to the take (and, for a punch, to `E − S`): a partial take commits what it holds. An empty
    /// window cancels the take (`take_cancel`, `Ok(None)`). Retry-safe like
    /// [`Self::commit_take`]: on error the take stays open.
    pub fn commit_take_window(
        &mut self,
        finished: &FinishedTake,
        window: (u64, u64),
        markers: &[Marker],
    ) -> Result<Option<HistoryStep>> {
        let open = match self.open_take {
            Some(open) if open.id == finished.take => open,
            _ => return Err(ProjectError::NoSuchTake(finished.take.0)),
        };
        let len = finished.audio.len_samples;
        if len == 0 && finished.wav_samples > 0 {
            return Err(ProjectError::TakeNotInStore {
                take: open.id.0,
                wav_samples: finished.wav_samples,
            });
        }
        let Some(mut edit) = self.build_take_edit(&open.plan, &finished.audio.pieces, window)?
        else {
            self.cancel_take(open.id)?;
            return Ok(None);
        };
        let n = Self::window_len(&open.plan, len, window);
        for op in self.take_marker_ops(&open.plan, n, markers) {
            edit = edit.marker(op);
        }
        let truncated = (finished.wav_samples > len).then_some(finished.wav_samples);
        let step = self.commit_internal(&edit, Some((open.id, truncated)))?;
        self.open_take = None;
        self.open_take_markers.clear();
        Ok(Some(step))
    }

    /// Samples of `window` that a take of `take_len` samples commits under `plan`.
    fn window_len(plan: &TakePlan, take_len: u64, window: (u64, u64)) -> u64 {
        let start = window.0.min(take_len);
        let n = window.1.min(take_len).saturating_sub(start);
        if plan.mode == TakeModeRecord::Punch {
            n.min(plan.len)
        } else {
            n
        }
    }

    /// The edit a record window becomes (SPEC-022 §4.1); `None` when the window is empty.
    fn build_take_edit(
        &self,
        plan: &TakePlan,
        take_pieces: &[Piece],
        window: (u64, u64),
    ) -> Result<Option<Edit>> {
        let take_len: u64 = take_pieces.iter().map(Piece::len_samples).sum();
        let n = Self::window_len(plan, take_len, window);
        if n == 0 {
            return Ok(None);
        }
        let ws = window.0.min(take_len);
        let cur = self.current();
        let at = plan.at.min(cur.len_samples);
        let take = DocSnapshot::new(self.sample_rate_hz, take_pieces.to_vec(), Vec::new());
        let edit = match plan.mode {
            TakeModeRecord::New | TakeModeRecord::Insert => {
                Edit::new(TAKE_LABEL_KEY).replace(at, 0, take.slice(ws, n)?)
            }
            TakeModeRecord::Overwrite => {
                // §4.2: fade-in at `c` only when `c < L`, fade-out only when `c + n < L`.
                let l = cur.len_samples;
                let x = plan.xfade_samples;
                let xs = if at < l { x.min(l - at).min(n / 2) } else { 0 };
                let xe = if at + n < l { x.min(n / 2) } else { 0 };
                let mut pieces = self.render_fade(&cur, at, &take, ws, xs, FadeDir::In)?;
                pieces.extend(take.slice(ws + xs, n - xs - xe)?);
                pieces.extend(self.render_fade(
                    &cur,
                    at + n - xe,
                    &take,
                    ws + n - xe,
                    xe,
                    FadeDir::Out,
                )?);
                Edit::new(TAKE_LABEL_KEY).replace_with(
                    at,
                    n.min(l - at),
                    pieces,
                    MarkerMapping::Identity,
                )
            }
            TakeModeRecord::Punch => {
                // §4.2: `X′ = min(X, ⌊(E′ − S)/2⌋)` at both ends, inside `[S, E′)`.
                let x = plan.xfade_samples.min(n / 2);
                let mut pieces = self.render_fade(&cur, at, &take, ws, x, FadeDir::In)?;
                pieces.extend(take.slice(ws + x, n - 2 * x)?);
                pieces.extend(self.render_fade(
                    &cur,
                    at + n - x,
                    &take,
                    ws + n - x,
                    x,
                    FadeDir::Out,
                )?);
                Edit::new(PUNCH_LABEL_KEY).replace_with(at, n, pieces, MarkerMapping::Identity)
            }
        };
        Ok(Some(edit))
    }

    /// Renders one equal-power boundary crossfade of `len` samples into new chunks (SPEC-022
    /// §4.2): `θᵢ = (π/2)·(i + 0.5)/len`, `out = cos θ·F + sin θ·G` in f64, rounded once, with
    /// `F = A, G = T` for [`FadeDir::In`] and `F = T, G = A` for [`FadeDir::Out`]. Reads only
    /// `A[doc_pos, doc_pos + len)` and `T[take_pos, take_pos + len)`.
    fn render_fade(
        &self,
        doc: &DocSnapshot,
        doc_pos: u64,
        take: &DocSnapshot,
        take_pos: u64,
        len: u64,
        dir: FadeDir,
    ) -> Result<Vec<Piece>> {
        if len == 0 {
            return Ok(Vec::new());
        }
        let n = usize::try_from(len)
            .map_err(|_| ProjectError::InvalidArgument("crossfade too long"))?;
        let mut a = vec![0.0f32; n];
        self.store.read(doc, doc_pos, &mut a)?;
        let mut t = vec![0.0f32; n];
        self.store.read(take, take_pos, &mut t)?;
        let out: Vec<f32> = (0..n)
            .map(|i| {
                let theta = std::f64::consts::FRAC_PI_2 * (i as f64 + 0.5) / len as f64;
                let (f, g) = match dir {
                    FadeDir::In => (a[i], t[i]),
                    FadeDir::Out => (t[i], a[i]),
                };
                (theta.cos() * f64::from(f) + theta.sin() * f64::from(g)) as f32
            })
            .collect();
        let mut writer = self.store.writer();
        writer.append(&out)?;
        Ok(writer.finish()?.pieces)
    }

    /// Crash recovery's "Open as new document" (SPEC-004 §2.7): copies the open take's WAV into a
    /// new session under `sessions_dir` (one undoable "Record" edit on an empty document), then
    /// closes the take here with `take_discard`. `Ok(None)` (and nothing created) when the WAV
    /// holds no audio.
    pub fn open_take_as_new_session(
        &mut self,
        sessions_dir: &Path,
        store: StoreOptions,
    ) -> Result<Option<Session>> {
        let open = self.open_take.ok_or(ProjectError::NoSuchTake(0))?;
        let parts = crate::take::recover_take(&self.takes_dir(), open.id.0)?;
        let mut fresh = Session::create(
            sessions_dir,
            SessionConfig {
                sample_rate_hz: self.sample_rate_hz,
                source: None,
                store,
            },
        )?;
        let mut writer = fresh.chunk_writer();
        // T-304 (SPEC-022 §2.12): only the record window part of an operation's take (a take
        // whose aligned window never opened has none).
        let plan = open.plan;
        let built = crate::take::copy_take_into(&parts, &mut writer)
            .and_then(|_| writer.finish())
            .and_then(|audio| {
                let start = plan.k_start.unwrap_or(audio.len_samples);
                let n = Self::window_len(&plan, audio.len_samples, (start, u64::MAX));
                if n == 0 {
                    return Ok(false);
                }
                let take = DocSnapshot::new(self.sample_rate_hz, audio.pieces.clone(), Vec::new());
                let pieces = take.slice(start.min(audio.len_samples), n)?;
                fresh
                    .commit_edit(Edit::new(TAKE_LABEL_KEY).replace(0, 0, pieces))
                    .map(|_| true)
            })
            .and_then(|made| self.discard_take(open.id).map(|()| made));
        match built {
            Ok(true) => Ok(Some(fresh)),
            Ok(false) => {
                let _ = fresh.close();
                Ok(None)
            }
            Err(e) => {
                let _ = fresh.close();
                Err(e)
            }
        }
    }

    /// Closes the document after Save or Don't Save: journals `close`, releases the store and the
    /// lock, and deletes the session directory (SPEC-004 §2.8). Refused while recording and
    /// while a reader/writer still shares the store; then nothing is written and the session is
    /// handed back in [`CloseError::session`].
    pub fn close(mut self) -> std::result::Result<(), CloseError> {
        let refusal = if self.open_take.is_some() {
            Some(ProjectError::NotWhileRecording)
        } else if Arc::strong_count(&self.store) > 1 {
            Some(ProjectError::StoreInUse)
        } else {
            None
        };
        if let Some(error) = refusal {
            return Err(CloseError {
                error,
                session: Some(Box::new(self)),
            });
        }
        if let Err(error) = self.journal.append(&[Record::Close]) {
            return Err(CloseError {
                error,
                session: Some(Box::new(self)),
            });
        }
        let dir = self.dir.clone();
        drop(self);
        crate::gc::delete_session_dir(&dir).map_err(|error| CloseError {
            error,
            session: None,
        })
    }
}

/// Writes this process's pid/host/start time into a session lock file (informational).
pub(crate) fn write_lock_info(lock: &File) -> Result<()> {
    let info = format!(
        "pid={}\nhost={}\nstart_unix_ms={}\n",
        std::process::id(),
        hostname(),
        unix_ms(SystemTime::now())
    );
    lock.set_len(0)
        .and_then(|()| write_all_at(lock, info.as_bytes(), 0))
        .map_err(ProjectError::io("writing the session lock"))
}

/// The generation-numbered files of a session directory.
const GENERATION_FILES: [(&str, &str); 3] = [
    ("chunks.", ".f32"),
    ("peaks.", ".bin"),
    ("journal.", ".jsonl"),
];

/// Removes generation `generation`'s store, peaks and journal files (best-effort).
pub(crate) fn remove_generation_files(dir: &Path, generation: u32) {
    for (prefix, suffix) in GENERATION_FILES {
        let _ = fs::remove_file(dir.join(format!("{prefix}{generation}{suffix}")));
    }
}

/// Removes every generation's files except `keep`'s (leftovers of an interrupted compaction).
pub(crate) fn remove_other_generations(dir: &Path, keep: u32) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        let generation = GENERATION_FILES.iter().find_map(|(prefix, suffix)| {
            name.strip_prefix(prefix)?
                .strip_suffix(suffix)?
                .parse::<u32>()
                .ok()
        });
        if generation.is_some_and(|g| g != keep) {
            let _ = fs::remove_file(entry.path());
        }
    }
}

fn create_unique_dir(sessions_dir: &Path) -> Result<(String, PathBuf)> {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    loop {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let id = format!("{}-{}-{n}", unix_ms(SystemTime::now()), std::process::id());
        let dir = sessions_dir.join(&id);
        match fs::create_dir(&dir) {
            Ok(()) => return Ok((id, dir)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(ProjectError::from_io("creating the session directory", e)),
        }
    }
}

fn hostname() -> String {
    #[cfg(target_os = "linux")]
    if let Ok(name) = fs::read_to_string("/proc/sys/kernel/hostname") {
        return name.trim().to_owned();
    }
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_default()
}

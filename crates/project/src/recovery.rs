//! Crash recovery (ADR-004 §9, SPEC-004 §2.7; T-301).
//!
//! [`replay`] rebuilds a document's exact state — current snapshot, undo/redo stacks with labels
//! and attachments, saved seq, counters — from journal records, starting at the last checkpoint.
//! It stops at the first record that doesn't apply cleanly (an edit whose seq or pieces don't
//! match, an undo/redo of the wrong entry): everything from there on counts as lost.
//! [`crate::Session::recover`] adds the file work: lock, scan the journal (stopping at the first
//! torn line or bad CRC), reopen the chunk store, verify every referenced chunk's CRC32, fall back
//! to the newest state whose audio is intact, truncate the journal there and hand back a live
//! session.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use crate::budget::live_chunk_ids;
use crate::gc::try_lock_session;
use crate::history::{Edit, EditOp, History};
use crate::journal::TakeModeRecord;
use crate::journal::{Journal, Record, SourceRecord, journal_file_name, scan_journal};
use crate::session::{
    META_FILE_NAME, OpenTake, SourceInfo, TAKES_DIR_NAME, TakeId, TakePlan, read_meta,
    remove_other_generations, write_lock_info,
};
use crate::snapshot::Source;
use crate::store::{ChunkId, ChunkLocation, ChunkStore, StoreOptions};
use crate::{ProjectError, Result, Session};

/// The take that was being recorded when the session ended (a `take_begin` with neither its
/// edit nor a `take_discard`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RecoveredTakeInfo {
    pub take: TakeId,
    /// Where "Apply as recorded" inserts it (clamped to the document length).
    pub at_samples: u64,
    /// Samples recoverable from its WAV part files.
    pub samples: u64,
}

/// The file a document was last saved to (journal `saved`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SavedFacts {
    pub path: PathBuf,
    /// `wav16` / `wav24` / `wav32f`.
    pub format: String,
    pub file_size_bytes: Option<u64>,
    pub file_mtime_unix_ms: Option<u64>,
}

/// What [`Session::recover`] found besides the document itself.
#[derive(Clone, Debug)]
pub struct RecoveryReport {
    /// Changes (edits, undos, redos) that could not be recovered: damaged or inconsistent
    /// journal records, or states whose audio failed its checksum ("The last N changes could
    /// not be recovered", SPEC-004 §2.7). 0 when nothing was lost.
    pub lost_changes: usize,
    /// The interrupted take, if any; it is still open on the recovered session until the caller
    /// applies it ([`Session::apply_open_take_from_wav`]), opens it as a new document
    /// ([`Session::open_take_as_new_session`]) or discards it ([`Session::discard_take`]).
    pub open_take: Option<RecoveredTakeInfo>,
    /// The latest journaled rack/view `state` (ADR-004 §6), opaque.
    pub state: Option<serde_json::Value>,
    /// The file the session was opened from (`None`: a new recording).
    pub source: Option<SourceInfo>,
    /// The latest save, if any.
    pub saved: Option<SavedFacts>,
}

impl RecoveryReport {
    /// The path the recovered document is bound to: its last save target, else the file it was
    /// opened from.
    pub fn bound_path(&self) -> Option<&Path> {
        self.saved
            .as_ref()
            .map(|s| s.path.as_path())
            .or_else(|| self.source.as_ref().map(|s| s.path.as_path()))
    }

    /// The `(size, mtime)` PowerVoice last knew the bound file to have.
    pub fn bound_file_facts(&self) -> Option<(u64, u64)> {
        match &self.saved {
            Some(saved) => saved.file_size_bytes.zip(saved.file_mtime_unix_ms),
            None => self
                .source
                .as_ref()
                .map(|s| (s.size_bytes, s.mtime_unix_ms)),
        }
    }
}

/// A document state rebuilt from journal records ([`replay`]).
#[derive(Debug)]
pub struct Replay {
    pub history: History,
    /// The chunk index as of the replayed records.
    pub chunks: BTreeMap<ChunkId, ChunkLocation>,
    pub source: Option<SourceRecord>,
    pub saved: Option<SavedFacts>,
    /// The latest `saved` / `state` records, verbatim.
    pub saved_record: Option<Record>,
    pub state_record: Option<Record>,
    /// Open takes: take id → what `take_begin` (+ `take_window`, T-304) journaled about it.
    pub open_takes: BTreeMap<u32, TakePlan>,
    /// Highest take id ever begun (0: none).
    pub max_take: u32,
    /// The journal ends in `close`.
    pub closed: bool,
}

/// [`replay`]'s result.
#[derive(Debug)]
pub struct ReplayOutcome {
    /// `None` when the records hold no checkpoint (nothing to rebuild from).
    pub replay: Option<Replay>,
    /// Records applied (a prefix of the input).
    pub consumed: usize,
    /// Change records (edit/undo/redo/drop) after the consumed prefix.
    pub lost_changes: usize,
}

fn is_change(record: &Record) -> bool {
    matches!(
        record,
        Record::Edit(_) | Record::Undo { .. } | Record::Redo { .. } | Record::DropUndo { .. }
    )
}

fn saved_facts(record: &Record) -> Option<SavedFacts> {
    match record {
        Record::Saved {
            path,
            format,
            file_size_bytes,
            file_mtime_unix_ms,
            ..
        } => Some(SavedFacts {
            path: PathBuf::from(path),
            format: format.clone(),
            file_size_bytes: *file_size_bytes,
            file_mtime_unix_ms: *file_mtime_unix_ms,
        }),
        _ => None,
    }
}

fn pieces_known(edit: &Edit, chunks: &BTreeMap<ChunkId, ChunkLocation>) -> bool {
    edit.ops.iter().all(|op| {
        let EditOp::Replace { pieces, .. } = op;
        pieces.iter().all(|p| match p.source {
            Source::Chunk(id) => chunks
                .get(&id)
                .is_some_and(|l| u64::from(p.offset) + u64::from(p.len) <= u64::from(l.len)),
            Source::Silence => true,
        })
    })
}

/// Rebuilds the document state from `records` (a journal's valid prefix): metadata from every
/// record, history from the last checkpoint on. Stops at the first record that does not apply
/// exactly as it did in the live session. Never panics on any input.
pub fn replay(records: &[Record]) -> ReplayOutcome {
    let Some(cp) = records
        .iter()
        .rposition(|r| matches!(r, Record::Checkpoint(_)))
    else {
        return ReplayOutcome {
            replay: None,
            consumed: 0,
            lost_changes: records.iter().filter(|r| is_change(r)).count(),
        };
    };
    let Record::Checkpoint(checkpoint) = &records[cp] else {
        unreachable!("rposition matched a checkpoint");
    };
    let mut r = Replay {
        history: checkpoint.to_history(),
        chunks: checkpoint.chunks.iter().map(|l| (l.id, *l)).collect(),
        source: None,
        saved: None,
        saved_record: None,
        state_record: None,
        open_takes: BTreeMap::new(),
        max_take: 0,
        closed: false,
    };
    // Metadata and take bookkeeping from before the checkpoint (its history already folds in
    // every edit, undo, redo and save up to it).
    for record in &records[..cp] {
        note_metadata(&mut r, record);
    }
    let mut consumed = cp + 1;
    for record in &records[cp + 1..] {
        if !apply(&mut r, record) {
            break;
        }
        consumed += 1;
    }
    let lost_changes = records[consumed..].iter().filter(|r| is_change(r)).count();
    ReplayOutcome {
        replay: Some(r),
        consumed,
        lost_changes,
    }
}

/// Bookkeeping every record contributes regardless of where it lies.
fn note_metadata(r: &mut Replay, record: &Record) {
    r.closed = matches!(record, Record::Close);
    match record {
        Record::Open { source, .. } => r.source = source.clone(),
        Record::Saved { .. } => {
            r.saved = saved_facts(record);
            r.saved_record = Some(record.clone());
        }
        Record::State { .. } => r.state_record = Some(record.clone()),
        Record::TakeBegin {
            take,
            mode,
            at,
            len,
            xfade_samples,
            offset_ns,
            aligned,
            ..
        } => {
            r.open_takes.insert(
                *take,
                TakePlan {
                    mode: *mode,
                    at: *at,
                    len: *len,
                    xfade_samples: *xfade_samples,
                    offset_ns: *offset_ns,
                    aligned: *aligned,
                    k_start: (!*aligned).then_some(0),
                },
            );
            r.max_take = r.max_take.max(*take);
        }
        Record::TakeWindow { take, k_start } => {
            if let Some(plan) = r.open_takes.get_mut(take) {
                plan.k_start = Some(*k_start);
            }
        }
        Record::TakeDiscard { take } | Record::TakeCancel { take } => {
            r.open_takes.remove(take);
        }
        Record::Edit(e) => {
            if let Some(take) = e.take {
                r.open_takes.remove(&take);
            }
        }
        _ => {}
    }
}

/// Applies one record after the checkpoint; `false` if it doesn't apply cleanly.
fn apply(r: &mut Replay, record: &Record) -> bool {
    match record {
        Record::Chunks { chunks } => {
            for loc in chunks {
                r.chunks.insert(loc.id, *loc);
            }
        }
        Record::Edit(e) => {
            let edit = e.to_edit();
            if !pieces_known(&edit, &r.chunks) {
                return false;
            }
            let Ok(prepared) = r.history.prepare(&edit) else {
                return false;
            };
            if prepared.seq() != e.seq {
                return false;
            }
            r.history.commit(prepared);
        }
        Record::Undo { seq } => {
            if r.history.peek_undo_seq() != Some(*seq) {
                return false;
            }
            r.history.undo();
        }
        Record::Redo { seq } => {
            if r.history.peek_redo_seq() != Some(*seq) {
                return false;
            }
            r.history.redo();
        }
        Record::DropUndo { count } => {
            let count = usize::try_from(*count).unwrap_or(usize::MAX);
            r.history.drop_oldest_undo(count);
        }
        Record::Saved { seq, .. } => r.history.set_saved_seq(*seq),
        Record::Checkpoint(_) => return false,
        Record::Open { .. }
        | Record::State { .. }
        | Record::TakeBegin { .. }
        | Record::TakeDiscard { .. }
        | Record::TakeWindow { .. }
        | Record::TakeCancel { .. }
        | Record::Close => {}
    }
    note_metadata(r, record);
    true
}

/// `true` if `record` introduces or references one of `ids`.
fn references_any(record: &Record, ids: &BTreeSet<ChunkId>) -> bool {
    match record {
        Record::Chunks { chunks } => chunks.iter().any(|l| ids.contains(&l.id)),
        Record::Edit(e) => e.to_edit().ops.iter().any(|op| {
            let EditOp::Replace { pieces, .. } = op;
            pieces
                .iter()
                .any(|p| matches!(p.source, Source::Chunk(id) if ids.contains(&id)))
        }),
        _ => false,
    }
}

/// Highest take id with a file in `takes_dir` (0: none).
fn max_take_file(takes_dir: &Path) -> u32 {
    let Ok(entries) = fs::read_dir(takes_dir) else {
        return 0;
    };
    entries
        .flatten()
        .filter_map(|e| {
            let name = e.file_name();
            let rest = name.to_str()?.strip_prefix("take-")?;
            rest.get(..4)?.parse::<u32>().ok()
        })
        .max()
        .unwrap_or(0)
}

impl Session {
    /// Recovers an unlocked session directory after a crash (ADR-004 §9 step 4, SPEC-004 §2.7):
    ///
    /// 1. locks it ([`ProjectError::SessionLocked`] if another instance holds it);
    /// 2. reads `meta.json` and the current generation's journal, keeping everything up to the
    ///    first torn line or bad CRC;
    /// 3. replays it from the last checkpoint ([`replay`]);
    /// 4. reopens the chunk store and verifies the CRC32 of every chunk the rebuilt history
    ///    references; if one fails, recovery falls back to the newest state before that chunk
    ///    entered the journal;
    /// 5. truncates the journal after the last record it kept and removes other generations'
    ///    leftover files.
    ///
    /// The document comes back exactly as of the last kept record: current snapshot, undo/redo
    /// stacks, labels, attachments, saved seq. An interrupted take stays open (see
    /// [`RecoveryReport::open_take`]). [`ProjectError::NothingRecoverable`] when there is no
    /// intact checkpoint or the undo floor's own audio is damaged. Never panics on damaged data.
    pub fn recover(dir: &Path, store: StoreOptions) -> Result<(Session, RecoveryReport)> {
        let lock = try_lock_session(dir)?.ok_or(ProjectError::SessionLocked)?;
        let meta = read_meta(dir).ok_or(ProjectError::NothingRecoverable)?;
        let generation = meta.generation;
        let journal_path = dir.join(journal_file_name(generation));
        let bytes = match fs::read(&journal_path) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(ProjectError::NothingRecoverable);
            }
            Err(e) => return Err(ProjectError::from_io("reading the journal", e)),
        };
        let scan = scan_journal(&bytes);
        let outcome = replay(&scan.records);
        let mut rep = outcome.replay.ok_or(ProjectError::NothingRecoverable)?;
        let mut consumed = outcome.consumed;
        let mut lost = outcome.lost_changes + scan.damaged_changes();

        let locations: Vec<ChunkLocation> = rep.chunks.values().copied().collect();
        let chunk_store = ChunkStore::open_existing(dir, generation, store.clone(), &locations)?;
        let corrupt: BTreeSet<ChunkId> = live_chunk_ids(&rep.history, &[])
            .into_iter()
            .filter(|&id| chunk_store.verify_chunk(id).is_err())
            .collect();
        if !corrupt.is_empty() {
            // The newest state whose audio is intact: everything before the first record that
            // brought a damaged chunk in.
            let cp = scan.records[..consumed]
                .iter()
                .rposition(|r| matches!(r, Record::Checkpoint(_)))
                .ok_or(ProjectError::NothingRecoverable)?;
            let cut = (cp + 1..consumed)
                .find(|&i| references_any(&scan.records[i], &corrupt))
                .ok_or(ProjectError::NothingRecoverable)?;
            let again = replay(&scan.records[..cut]);
            rep = again.replay.ok_or(ProjectError::NothingRecoverable)?;
            if live_chunk_ids(&rep.history, &[])
                .iter()
                .any(|id| corrupt.contains(id))
            {
                return Err(ProjectError::NothingRecoverable);
            }
            lost += scan.records[cut..consumed]
                .iter()
                .filter(|r| is_change(r))
                .count();
            consumed = again.consumed;
        }

        let keep_bytes = consumed.checked_sub(1).map_or(0, |i| scan.ends[i]);
        let journal = Journal::open_truncated(&journal_path, keep_bytes)?;
        write_lock_info(&lock)?;
        remove_other_generations(dir, generation);
        let _ = fs::remove_file(dir.join(format!("{META_FILE_NAME}.tmp")));

        let takes_dir = dir.join(TAKES_DIR_NAME);
        let last_take = rep
            .open_takes
            .iter()
            .next_back()
            .map(|(&id, &plan)| OpenTake {
                id: TakeId(id),
                plan,
            });
        // T-304 (SPEC-022 §2.12, §4.6): an aligned take whose record window never opened (a
        // crash during pre-roll) holds nothing applicable: it is cancelled below, not offered.
        let (open_take, cancelled_take) = match last_take {
            Some(t) if t.plan.aligned && t.plan.k_start.is_none() => (None, Some(t.id)),
            other => (other, None),
        };
        let open_take_info = match open_take {
            Some(t) => {
                let total: u64 = crate::take::recover_take(&takes_dir, t.id.0)
                    .map(|parts| parts.iter().map(|p| p.samples).sum())
                    .unwrap_or(0);
                // What "Apply as recorded" commits: the record window part of the take.
                let mut samples = total.saturating_sub(t.plan.k_start.unwrap_or(0));
                if t.plan.mode == TakeModeRecord::Punch {
                    samples = samples.min(t.plan.len);
                }
                Some(RecoveredTakeInfo {
                    take: t.id,
                    at_samples: t.plan.at.min(rep.history.current().len_samples),
                    samples,
                })
            }
            None => None,
        };
        let next_take = rep.max_take.max(max_take_file(&takes_dir)) + 1;
        let source = rep.source.as_ref().map(|s| SourceInfo {
            path: PathBuf::from(&s.path),
            size_bytes: s.size_bytes,
            mtime_unix_ms: s.mtime_unix_ms,
            format: s.format.clone(),
        });
        let report = RecoveryReport {
            lost_changes: lost,
            open_take: open_take_info,
            state: match &rep.state_record {
                Some(Record::State { state }) => Some(state.clone()),
                _ => None,
            },
            source,
            saved: rep.saved.clone(),
        };
        let open_record = Record::Open {
            format_version: meta.format_version,
            sample_rate_hz: meta.sample_rate_hz,
            source: rep.source.clone(),
        };
        let mut session = Session {
            id: dir
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            dir: dir.to_path_buf(),
            _lock: lock,
            store: chunk_store,
            journal,
            history: rep.history,
            sample_rate_hz: meta.sample_rate_hz,
            journaled_chunks: Vec::new(),
            open_take,
            next_take,
            generation,
            store_options: store,
            meta,
            open_record,
            last_saved: rep.saved_record,
            last_state: rep.state_record,
        };
        for &id in rep.chunks.keys() {
            session.mark_journaled(id);
        }
        if let Some(take) = cancelled_take {
            session
                .journal
                .append(&[Record::TakeCancel { take: take.0 }])?;
        }
        Ok((session, report))
    }
}

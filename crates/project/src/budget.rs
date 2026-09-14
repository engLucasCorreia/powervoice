//! Disk budget, compaction and dropping the oldest undo steps (SPEC-004 §2.5, ADR-004 §4; T-301).
//!
//! - [`Session::disk_usage`] measures the session: its directory size, the chunk file, and how
//!   many chunk bytes the current document, the history and nothing at all reference.
//! - [`decide`] is the pure policy: nothing while under the limits; compaction first when
//!   ≥ 25 % of the chunk data is unreachable (or compaction alone gets back under the limit);
//!   otherwise drop the fewest oldest undo entries that get under it (OD-1 = A), then compact;
//!   a "disk almost full" warning when free space stays under the floor with nothing left.
//! - [`Session::compact`] copies the live chunks into the next generation (ids kept), writes its
//!   journal starting with a checkpoint, switches `meta.json` atomically and deletes the old
//!   generation — crash-safe at every step. H-17 item 3: [`Session::begin_compact`] +
//!   [`PreparedCompaction::copy_chunks`] + [`Session::finish_compact`] split that into a short
//!   snapshot, the expensive copy (needs only `&Session`, so it never blocks edits), and a short
//!   swap; [`Session::compact`] is just those three steps run back to back.
//! - [`Session::housekeep`] runs the three together. Nothing here runs while recording.

use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

use crate::disk::FreeSpaceProvider;
use crate::fs_util::{dir_size, write_file_atomic};
use crate::history::History;
use crate::journal::{CheckpointRecord, Journal, Record, journal_file_name};
use crate::session::{META_FILE_NAME, Meta, read_meta, remove_generation_files};
use crate::snapshot::{DocSnapshot, Piece, Source};
use crate::store::{ChunkId, ChunkLocation, ChunkStore};
use crate::{CHUNK_ALIGN_BYTES, CHUNK_SAMPLES, ProjectError, Result, SEGMENT_BYTES, Session};

const GIB: u64 = 1024 * 1024 * 1024;
/// Lower bound of the session's soft limit (SPEC-004 §3 `disk_soft_limit`).
pub const MIN_SOFT_LIMIT_BYTES: u64 = 8 * GIB;
/// The soft limit is at least this many times the document's size.
pub const SOFT_LIMIT_DOCUMENT_FACTOR: u64 = 8;
/// Minimum free space on the volume (SPEC-004 §3 `disk_free_floor`).
pub const DISK_FREE_FLOOR_BYTES: u64 = 2 * GIB;
/// Compact first when at least this share of the chunk data is unreachable (`compact_threshold`).
pub const COMPACT_THRESHOLD_PCT: u64 = 25;

/// `max(8 GiB, 8 × document bytes)` (SPEC-004 §2.5).
pub fn soft_limit_bytes(document_bytes: u64) -> u64 {
    MIN_SOFT_LIMIT_BYTES.max(document_bytes.saturating_mul(SOFT_LIMIT_DOCUMENT_FACTOR))
}

/// The limits [`decide`] checks. `Default` is SPEC-004's: the formula soft limit and a 2 GiB free
/// floor; tests (and AC-4's "limit raised to 100 GiB") override the soft limit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DiskLimits {
    /// `None`: [`soft_limit_bytes`] of the current document.
    pub soft_limit_bytes: Option<u64>,
    pub free_floor_bytes: u64,
}

impl Default for DiskLimits {
    fn default() -> Self {
        DiskLimits {
            soft_limit_bytes: None,
            free_floor_bytes: DISK_FREE_FLOOR_BYTES,
        }
    }
}

/// A session's disk use ([`Session::disk_usage`]). Chunk bytes count each chunk's 4 KiB-aligned
/// footprint in the chunk file.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DiskUsage {
    /// Everything in the session directory ("Session storage", SPEC-004 §2.5).
    pub session_bytes: u64,
    /// The chunk file (whole preallocated segments).
    pub chunk_file_bytes: u64,
    /// Chunks the current document references.
    pub document_bytes: u64,
    /// Chunks only the undo/redo history (or the clipboard) references ("history").
    pub history_bytes: u64,
    /// Committed chunks nothing references any more (reclaimed by compaction).
    pub unreachable_bytes: u64,
    /// `live_after_drop[k]`: live chunk bytes once the `k` oldest undo entries are dropped
    /// (`k = 0 ..= undo depth`; `[0]` = document + history).
    pub live_after_drop: Vec<u64>,
}

/// What [`decide`] asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiskAction {
    /// Within the limits (or nothing to do).
    None,
    /// Reclaim unreachable chunks (silent).
    Compact,
    /// Drop this many oldest undo entries, then compact (with a notice).
    DropOldest(usize),
    /// Free space is under the floor and nothing is left to reclaim: warn persistently.
    AlmostFull,
}

/// SPEC-004 §2.5 / AC-13's policy (see the module docs).
pub fn decide(usage: &DiskUsage, free_bytes: u64, limits: &DiskLimits) -> DiskAction {
    let soft = limits
        .soft_limit_bytes
        .unwrap_or_else(|| soft_limit_bytes(usage.document_bytes));
    let floor = limits.free_floor_bytes;
    if usage.session_bytes <= soft && free_bytes >= floor {
        return DiskAction::None;
    }
    let live0 = usage.live_after_drop.first().copied().unwrap_or(0);
    let compacted_file = |live: u64| live.next_multiple_of(SEGMENT_BYTES);
    let compaction_shrinks = usage.chunk_file_bytes > compacted_file(live0);
    let total = live0 + usage.unreachable_bytes;
    if compaction_shrinks
        && total > 0
        && usage.unreachable_bytes * 100 >= total * COMPACT_THRESHOLD_PCT
    {
        return DiskAction::Compact;
    }
    let other = usage.session_bytes.saturating_sub(usage.chunk_file_bytes);
    let fits = |live: u64| {
        let after = other + compacted_file(live);
        let freed = usage.session_bytes.saturating_sub(after);
        after <= soft && free_bytes.saturating_add(freed) >= floor
    };
    match usage.live_after_drop.iter().position(|&live| fits(live)) {
        Some(0) if compaction_shrinks => return DiskAction::Compact,
        Some(0) => {}
        Some(k) => return DiskAction::DropOldest(k),
        None => {}
    }
    let depth = usage.live_after_drop.len().saturating_sub(1);
    if depth > 0 && usage.live_after_drop[depth] < live0 {
        return DiskAction::DropOldest(depth);
    }
    if compaction_shrinks && usage.unreachable_bytes > 0 {
        return DiskAction::Compact;
    }
    if free_bytes < floor {
        DiskAction::AlmostFull
    } else {
        DiskAction::None
    }
}

/// What [`Session::housekeep`] did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HousekeepingReport {
    /// A take is open: nothing ran (SPEC-004 §2.5 "never while recording").
    pub skipped_recording: bool,
    /// The store moved to a new generation: hand the new store to readers.
    pub compacted: bool,
    /// Oldest undo entries removed (the notice's count).
    pub dropped_undo: usize,
    /// Chunk-file bytes released (the notice's size).
    pub freed_bytes: u64,
    /// Free space stays under the floor with nothing left to reclaim.
    pub almost_full: bool,
}

/// H-17 item 3: a compaction snapshotted by [`Session::begin_compact`]. Its expensive part —
/// reading every live chunk from the old store and writing it into the new generation's
/// ([`Self::copy_chunks`]) — touches neither `self` nor the [`Session`] it came from, so it can
/// run for as long as it takes (a whole document's worth of I/O) while the session keeps
/// committing edits against the old store. [`Session::finish_compact`] re-checks what's live,
/// copies the (usually empty) delta, and does the short atomic generation switch.
pub struct PreparedCompaction {
    old_store: Arc<ChunkStore>,
    new_store: Arc<ChunkStore>,
    live: BTreeSet<ChunkId>,
    new_generation: u32,
}

impl PreparedCompaction {
    /// Reads every chunk snapshotted as live from the old store and writes it into the new one.
    /// Pure I/O against two independent chunk stores — no session state is touched, so nothing
    /// needs to wait for it (H-17 item 3).
    pub fn copy_chunks(&self) -> Result<Vec<ChunkLocation>> {
        copy_live_chunks(&self.old_store, &self.new_store, &self.live)
    }
}

/// Reads every chunk in `ids` from `old` and writes it into `new`, keeping its id.
fn copy_live_chunks(
    old: &ChunkStore,
    new: &ChunkStore,
    ids: &BTreeSet<ChunkId>,
) -> Result<Vec<ChunkLocation>> {
    let mut buf = vec![0f32; CHUNK_SAMPLES];
    let mut index = Vec::with_capacity(ids.len());
    for &id in ids {
        let loc = old.location(id).ok_or(ProjectError::UnknownChunk(id))?;
        let samples = &mut buf[..loc.len as usize];
        old.read_chunk(id, 0, samples)?;
        index.push(new.commit_chunk_with_id(id, samples)?);
    }
    Ok(index)
}

fn chunk_ids(snapshot: &DocSnapshot) -> impl Iterator<Item = ChunkId> + '_ {
    pieces_chunk_ids(&snapshot.pieces)
}

fn pieces_chunk_ids(pieces: &[Piece]) -> impl Iterator<Item = ChunkId> + '_ {
    pieces.iter().filter_map(|p| match p.source {
        Source::Chunk(id) => Some(id),
        Source::Silence => None,
    })
}

/// Chunk ids the current snapshot, every undo/redo entry and `extra` (the clipboard) reference.
pub(crate) fn live_chunk_ids(history: &History, extra: &[Piece]) -> BTreeSet<ChunkId> {
    let mut ids: BTreeSet<ChunkId> = chunk_ids(history.current()).collect();
    for entry in history.undo_entries().iter().chain(history.redo_entries()) {
        ids.extend(chunk_ids(&entry.snapshot));
    }
    ids.extend(pieces_chunk_ids(extra));
    ids
}

fn chunk_cost(store: &ChunkStore, id: ChunkId) -> u64 {
    store.location(id).map_or(0, |l| {
        (u64::from(l.len) * 4).next_multiple_of(CHUNK_ALIGN_BYTES)
    })
}

impl Session {
    /// Measures the session's disk use; `extra_live` (the clipboard) counts as live history.
    pub fn disk_usage(&self, extra_live: &[Piece]) -> DiskUsage {
        const ALWAYS: usize = usize::MAX;
        let undo = self.history.undo_entries();
        // For each live chunk: `ALWAYS` if the current state, the redo stack or `extra` needs it,
        // else 1 + the index of the newest undo entry that does (dropping the `k` oldest entries
        // frees it iff its level is ≤ k).
        let mut level: HashMap<ChunkId, usize> = HashMap::new();
        for id in chunk_ids(self.history.current())
            .chain(
                self.history
                    .redo_entries()
                    .iter()
                    .flat_map(|e| chunk_ids(&e.snapshot)),
            )
            .chain(pieces_chunk_ids(extra_live))
        {
            level.insert(id, ALWAYS);
        }
        for (i, entry) in undo.iter().enumerate() {
            for id in chunk_ids(&entry.snapshot) {
                let l = level.entry(id).or_insert(0);
                if *l != ALWAYS {
                    *l = (*l).max(i + 1);
                }
            }
        }
        let depth = undo.len();
        let mut always = 0u64;
        let mut by_level = vec![0u64; depth + 1];
        for (&id, &l) in &level {
            let cost = chunk_cost(&self.store, id);
            if l == ALWAYS {
                always += cost;
            } else {
                by_level[l] += cost;
            }
        }
        let mut live_after_drop = vec![0u64; depth + 1];
        let mut acc = always;
        for k in (0..=depth).rev() {
            live_after_drop[k] = acc;
            acc += by_level[k];
        }
        let document: BTreeSet<ChunkId> = chunk_ids(self.history.current()).collect();
        let document_bytes: u64 = document.iter().map(|&id| chunk_cost(&self.store, id)).sum();
        let live0 = live_after_drop[0];
        let all: u64 = self
            .store
            .locations()
            .iter()
            .map(|l| (u64::from(l.len) * 4).next_multiple_of(CHUNK_ALIGN_BYTES))
            .sum();
        DiskUsage {
            session_bytes: dir_size(&self.dir),
            chunk_file_bytes: self.store.chunk_file_bytes(),
            document_bytes,
            history_bytes: live0.saturating_sub(document_bytes),
            unreachable_bytes: all.saturating_sub(live0),
            live_after_drop,
        }
    }

    /// Removes the `count` oldest undo entries (SPEC-004 §2.5 OD-1 = A), journaled as
    /// `drop_undo` first. The current document and the redo stack are never touched. Refused
    /// while recording. Returns how many were removed; the space comes back with the next
    /// [`Self::compact`].
    pub fn drop_oldest_undo(&mut self, count: usize) -> Result<usize> {
        if self.open_take.is_some() {
            return Err(ProjectError::NotWhileRecording);
        }
        let n = count.min(self.history.undo_depth());
        if n == 0 {
            return Ok(0);
        }
        self.journal
            .append(&[Record::DropUndo { count: n as u64 }])?;
        Ok(self.history.drop_oldest_undo(n))
    }

    /// Compaction by generation (ADR-004 §4): copies every chunk the history or `extra_live`
    /// (the clipboard) references into `chunks.<gen+1>` keeping its id, writes `journal.<gen+1>`
    /// starting with a checkpoint, `fsync`s both, switches `meta.json` atomically, then deletes
    /// the old generation. A crash before the switch leaves the old generation authoritative
    /// (the new files are removed by the next compaction or recovery); after it, the new one.
    /// Refused while recording. Returns the chunk-file bytes released. [`Self::store`] is a new
    /// store afterwards: hand it to every reader.
    ///
    /// Just [`Self::begin_compact`] + [`PreparedCompaction::copy_chunks`] + [`Self::finish_compact`]
    /// run back to back — callers who want edits to keep committing while the (potentially slow,
    /// whole-document) copy runs should call those directly instead (H-17 item 3;
    /// `src-tauri`'s `DocumentService::housekeeping` does).
    pub fn compact(&mut self, extra_live: &[Piece]) -> Result<u64> {
        let prepared = self.begin_compact(extra_live)?;
        let index = prepared.copy_chunks()?;
        // `finish_compact` only returns `None` when `prepared` was snapshotted from a store this
        // session has since moved on from — impossible here, nothing runs between the two calls.
        Ok(self
            .finish_compact(prepared, index, extra_live)?
            .unwrap_or(0))
    }

    /// Phase 1 of compaction (short — no chunk I/O): snapshots what's live and opens the next
    /// generation's empty store. Needs only `&Session`: it does not touch `self` at all, so a
    /// caller can keep committing edits against the session in between this and
    /// [`Self::finish_compact`] without waiting on anything (H-17 item 3). Refused while
    /// recording.
    pub fn begin_compact(&self, extra_live: &[Piece]) -> Result<PreparedCompaction> {
        if self.open_take.is_some() {
            return Err(ProjectError::NotWhileRecording);
        }
        let live = live_chunk_ids(&self.history, extra_live);
        let new_generation = self.generation + 1;
        remove_generation_files(&self.dir, new_generation);
        let mut options = self.store_options.clone();
        options.memory_budget_bytes = self.store.memory_budget();
        let new_store = ChunkStore::create(&self.dir, new_generation, options)?;
        Ok(PreparedCompaction {
            old_store: Arc::clone(&self.store),
            new_store,
            live,
            new_generation,
        })
    }

    /// Phase 3 of compaction (short — no chunk I/O beyond `delta`, normally empty): re-snapshots
    /// what's live now — edits committed while [`PreparedCompaction::copy_chunks`] ran may have
    /// made new chunks live, or the disk-pressure caller may have dropped undo entries in between
    /// — copies just that delta, then does the same atomic generation switch [`Self::compact`]
    /// always did: checkpoint + `meta.json` swap + old generation removal. `index` is phase 2's
    /// copied locations. `Ok(None)` if `prepared` was snapshotted from a store this session has
    /// moved on from (recovered, or already compacted by a concurrent attempt) instead of doing
    /// anything — the caller's next housekeeping pass tries again; `Ok(Some(freed))` (`freed` may
    /// be 0) whenever the switch actually happened, so the caller knows to hand the new store to
    /// its readers. Refused while recording.
    ///
    /// Reserves the new store's ids below the old store's chunk count read *here* (not in
    /// [`Self::begin_compact`]): ids are never reused, and a chunk the old store hands out during
    /// the no-lock copy (e.g. an edit committed in the meantime) is still live and gets copied
    /// above by id, but the new store's own auto-assigned counter must still start past it (and
    /// past anything a job still holding the old store commits even later, ADR-004 Amendment 3) —
    /// a count read back in [`Self::begin_compact`] could already be stale by the time the copy
    /// finishes.
    pub fn finish_compact(
        &mut self,
        prepared: PreparedCompaction,
        mut index: Vec<ChunkLocation>,
        extra_live: &[Piece],
    ) -> Result<Option<u64>> {
        if self.open_take.is_some() {
            return Err(ProjectError::NotWhileRecording);
        }
        if !Arc::ptr_eq(&prepared.old_store, &self.store) {
            return Ok(None);
        }
        let live_now = live_chunk_ids(&self.history, extra_live);
        let delta: BTreeSet<ChunkId> = live_now.difference(&prepared.live).copied().collect();
        if !delta.is_empty() {
            index.extend(copy_live_chunks(&self.store, &prepared.new_store, &delta)?);
        }
        prepared
            .new_store
            .reserve_ids_below(self.store.chunk_count());
        let new_store = prepared.new_store;
        new_store.sync()?;
        let new_generation = prepared.new_generation;
        let old_generation = self.generation;
        let journal = match self.finish_generation_switch(index, new_generation) {
            Ok(journal) => journal,
            Err(e) => {
                if read_meta(&self.dir).is_some_and(|m| m.generation == new_generation) {
                    // The switch happened (only the directory sync failed): the new generation
                    // is authoritative now.
                    Journal::open_append(&self.dir.join(journal_file_name(new_generation)))?
                } else {
                    drop(new_store);
                    remove_generation_files(&self.dir, new_generation);
                    return Err(e);
                }
            }
        };
        let freed = self
            .store
            .chunk_file_bytes()
            .saturating_sub(new_store.chunk_file_bytes());
        self.store = new_store;
        self.journal = journal;
        self.generation = new_generation;
        self.meta.generation = new_generation;
        self.journaled_chunks.clear();
        for &id in prepared.live.union(&delta) {
            self.mark_journaled(id);
        }
        remove_generation_files(&self.dir, old_generation);
        Ok(Some(freed))
    }

    /// The checkpoint + `meta.json` half of the switch: journals `open` + the latest `saved`/
    /// `state` + a checkpoint built from `index` and the *current* `self.history` (so edits
    /// committed during the no-lock copy are included), then atomically points `meta.json` at
    /// `new_generation`.
    fn finish_generation_switch(
        &self,
        index: Vec<ChunkLocation>,
        new_generation: u32,
    ) -> Result<Journal> {
        let mut journal = Journal::create(&self.dir.join(journal_file_name(new_generation)))?;
        let mut records = vec![self.open_record.clone()];
        records.extend(self.last_saved.clone());
        records.extend(self.last_state.clone());
        records.push(Record::Checkpoint(CheckpointRecord::from_history(
            &self.history,
            index,
        )));
        journal.append(&records)?;
        let meta = Meta {
            generation: new_generation,
            ..self.meta.clone()
        };
        write_file_atomic(
            &self.dir.join(META_FILE_NAME),
            &serde_json::to_vec_pretty(&meta)?,
        )
        .map_err(ProjectError::io("switching the store generation"))?;
        Ok(journal)
    }

    /// SPEC-004 §2.5's check (after every committed edit and every 10 s idle): measures, asks
    /// [`decide`] and acts — compaction, then dropping the oldest undo steps — until within the
    /// limits (at most three rounds). Does nothing while recording.
    pub fn housekeep(
        &mut self,
        extra_live: &[Piece],
        free: &dyn FreeSpaceProvider,
        limits: &DiskLimits,
    ) -> Result<HousekeepingReport> {
        let mut report = HousekeepingReport::default();
        if self.open_take.is_some() {
            report.skipped_recording = true;
            return Ok(report);
        }
        for _ in 0..3 {
            let usage = self.disk_usage(extra_live);
            // Space released by this run may not show as free yet (readers still hold the old
            // store's file open), so credit it.
            let free_bytes = free
                .free_bytes(&self.dir)
                .unwrap_or(u64::MAX)
                .saturating_add(report.freed_bytes);
            match decide(&usage, free_bytes, limits) {
                DiskAction::None => break,
                DiskAction::AlmostFull => {
                    report.almost_full = true;
                    break;
                }
                DiskAction::Compact => {
                    report.freed_bytes += self.compact(extra_live)?;
                    report.compacted = true;
                }
                DiskAction::DropOldest(k) => {
                    report.dropped_undo += self.drop_oldest_undo(k)?;
                    report.freed_bytes += self.compact(extra_live)?;
                    report.compacted = true;
                }
            }
        }
        Ok(report)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIB: u64 = 1024 * 1024;
    const SEG: u64 = SEGMENT_BYTES;

    fn usage(session: u64, chunk_file: u64, unreachable: u64, live: Vec<u64>) -> DiskUsage {
        DiskUsage {
            session_bytes: session,
            chunk_file_bytes: chunk_file,
            document_bytes: live.last().copied().unwrap_or(0),
            history_bytes: 0,
            unreachable_bytes: unreachable,
            live_after_drop: live,
        }
    }

    fn limits(soft: u64) -> DiskLimits {
        DiskLimits {
            soft_limit_bytes: Some(soft),
            free_floor_bytes: 2 * GIB,
        }
    }

    #[test]
    fn soft_limit_is_max_of_8_gib_and_8x_the_document() {
        assert_eq!(soft_limit_bytes(0), 8 * GIB);
        assert_eq!(soft_limit_bytes(GIB), 8 * GIB);
        assert_eq!(soft_limit_bytes(2 * GIB), 16 * GIB);
    }

    #[test]
    fn under_the_limits_nothing_happens() {
        let u = usage(4 * SEG, 4 * SEG, 3 * SEG, vec![SEG]);
        assert_eq!(decide(&u, 100 * GIB, &limits(8 * SEG)), DiskAction::None);
    }

    /// AC-13 bullet 1: ≥ 25 % unreachable → compaction first, no undo removed.
    #[test]
    fn compaction_first_when_a_quarter_is_unreachable() {
        // 10 segments of chunks, 3 unreachable (30 %), limit 8 segments.
        let u = usage(10 * SEG, 10 * SEG, 3 * SEG, vec![7 * SEG, 5 * SEG, 2 * SEG]);
        assert_eq!(decide(&u, 100 * GIB, &limits(8 * SEG)), DiskAction::Compact);
    }

    /// AC-13 bullet 2: otherwise the fewest oldest undo entries that get under the limit.
    #[test]
    fn otherwise_drops_the_fewest_oldest_undo_entries() {
        // 1 segment unreachable (10 %); dropping 1 entry → 8 segments live (fits 8), not 7.
        let u = usage(
            10 * SEG,
            10 * SEG,
            SEG,
            vec![9 * SEG, 8 * SEG, 6 * SEG, 2 * SEG],
        );
        assert_eq!(
            decide(&u, 100 * GIB, &limits(8 * SEG)),
            DiskAction::DropOldest(1)
        );
        // A tighter limit needs two.
        assert_eq!(
            decide(&u, 100 * GIB, &limits(7 * SEG)),
            DiskAction::DropOldest(2)
        );
    }

    #[test]
    fn free_space_floor_counts_too() {
        // Within the soft limit, but only 1 GiB free: dropping must free ≥ 1 GiB.
        let u = usage(
            40 * SEG,
            40 * SEG,
            0,
            vec![40 * SEG, 30 * SEG, 20 * SEG, 10 * SEG],
        );
        let need = GIB / SEG; // segments per GiB
        let d = decide(&u, GIB, &limits(1000 * SEG));
        assert_eq!(d, DiskAction::DropOldest(2), "need {need} segments");
    }

    #[test]
    fn nothing_left_to_reclaim_warns_only_when_the_disk_is_nearly_full() {
        let u = usage(SEG, SEG, 0, vec![SEG]);
        assert_eq!(decide(&u, GIB, &limits(1000 * SEG)), DiskAction::AlmostFull);
        // Over the soft limit with plenty of free space and nothing to reclaim: no warning.
        assert_eq!(decide(&u, 100 * GIB, &limits(MIB)), DiskAction::None);
    }

    #[test]
    fn dropping_everything_is_the_last_resort_before_warning() {
        let u = usage(4 * SEG, 4 * SEG, 0, vec![4 * SEG, 3 * SEG, 2 * SEG]);
        assert_eq!(
            decide(&u, 100 * GIB, &limits(SEG)),
            DiskAction::DropOldest(2)
        );
    }
}

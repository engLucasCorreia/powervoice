//! Snapshot sample reader (ADR-004 §3–§4, ADR-002 §1 reader/prefetch thread).

use std::sync::Arc;

use crate::Result;
use crate::snapshot::{DocSnapshot, Source};
use crate::store::{ChunkStore, SegmentGuard};

/// Copies ranges of one snapshot into caller buffers. Owned by one non-real-time thread (the
/// engine's reader/prefetch thread, a save/export job, …), never by an audio thread: reads can
/// page-fault and verify CRCs.
///
/// The reader keeps a guard on the segment it read last. Sequential reads therefore don't touch
/// the segment table, and the segment backing the playback position can't be evicted
/// (ADR-004 §4). Call [`Self::release_segments`] when idle to make it evictable again.
pub struct SnapshotReader {
    store: Arc<ChunkStore>,
    snapshot: Arc<DocSnapshot>,
    cache: Option<SegmentGuard>,
}

impl SnapshotReader {
    /// A reader over `snapshot`.
    pub fn new(store: Arc<ChunkStore>, snapshot: Arc<DocSnapshot>) -> Self {
        SnapshotReader {
            store,
            snapshot,
            cache: None,
        }
    }

    /// The snapshot being read.
    pub fn snapshot(&self) -> &Arc<DocSnapshot> {
        &self.snapshot
    }

    /// Switches to another snapshot (after a destructive edit, ADR-004 §3) and returns the old
    /// one, so the caller controls on which thread it is dropped.
    pub fn set_snapshot(&mut self, snapshot: Arc<DocSnapshot>) -> Arc<DocSnapshot> {
        std::mem::replace(&mut self.snapshot, snapshot)
    }

    /// Document length in samples.
    pub fn len_samples(&self) -> u64 {
        self.snapshot.len_samples
    }

    /// Copies `[pos, pos + out.len())` into `out`, clipped at the end of the document; returns
    /// the number of samples written (0 at or past the end). Silence pieces read as `0.0`.
    pub fn read(&mut self, pos: u64, out: &mut [f32]) -> Result<usize> {
        read_range(&self.store, &self.snapshot, pos, out, &mut self.cache)
    }

    /// Drops the cached segment guard.
    pub fn release_segments(&mut self) {
        self.cache = None;
    }
}

pub(crate) fn read_range(
    store: &ChunkStore,
    snapshot: &DocSnapshot,
    pos: u64,
    out: &mut [f32],
    cache: &mut Option<SegmentGuard>,
) -> Result<usize> {
    let Some((mut i, mut in_piece)) = snapshot.locate(pos) else {
        return Ok(0);
    };
    let n = (out.len() as u64).min(snapshot.len_samples - pos) as usize;
    let mut done = 0;
    while done < n {
        let piece = snapshot.pieces[i];
        let take = (u64::from(piece.len) - in_piece).min((n - done) as u64) as usize;
        let dst = &mut out[done..done + take];
        match piece.source {
            Source::Silence => dst.fill(0.0),
            Source::Chunk(id) => {
                store.read_chunk_into(id, piece.offset + in_piece as u32, dst, cache)?;
            }
        }
        done += take;
        i += 1;
        in_piece = 0;
    }
    Ok(n)
}

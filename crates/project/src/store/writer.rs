//! [`ChunkWriter`]: streams new audio into the store as immutable chunks (ADR-004 §3).

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use crate::snapshot::{DocSnapshot, Piece, push_piece};
use crate::store::{ChunkId, ChunkStore};
use crate::{CHUNK_SAMPLES, ProjectError, Result};

/// Shared cancellation flag for a long-running writer (checked on every append).
#[derive(Clone, Debug, Default)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    /// A fresh, uncancelled token.
    pub fn new() -> Self {
        CancelToken::default()
    }

    /// Requests cancellation.
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Relaxed);
    }

    /// `true` once [`Self::cancel`] was called.
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}

/// Progress of a writer, readable from any thread.
#[derive(Clone, Debug, Default)]
pub struct WriterProgress(Arc<AtomicU64>);

impl WriterProgress {
    /// Samples accepted by the writer so far.
    pub fn samples_written(&self) -> u64 {
        self.0.load(Ordering::Relaxed)
    }
}

/// Audio committed to the store by a writer, as pieces ready to be spliced into a document.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WrittenAudio {
    pub pieces: Vec<Piece>,
    pub chunks: Vec<ChunkId>,
    pub len_samples: u64,
}

/// Records one commit into `audio` (a chunk's peaks/CRC are already durable by the time this
/// runs, ADR-004 §5 — only the piece-table/length bookkeeping happens here).
fn record_commit(audio: &mut WrittenAudio, id: ChunkId, len: u32) {
    push_piece(&mut audio.pieces, Piece::chunk(id, 0, len));
    audio.chunks.push(id);
    audio.len_samples += u64::from(len);
}

/// H-71 (SPEC-005 §2.3, ADR-003 Amendment 6): a read-only, thread-safe view of a [`ChunkWriter`]'s
/// [`WrittenAudio`] so far, updated on every chunk commit — [`ChunkWriter::track_live`]'s handle.
/// Lets another thread (the IPC command layer) query the growing session's already-committed
/// peaks while `import_file` is still decoding on its own thread, without touching `ChunkWriter`
/// itself (which stays `&mut`-owned by the importing thread throughout).
#[derive(Clone)]
pub struct LiveWrittenAudio(Arc<Mutex<WrittenAudio>>);

impl LiveWrittenAudio {
    /// Samples committed so far (a lower bound: more may land between this call and the next).
    pub fn len_samples(&self) -> u64 {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).len_samples
    }

    /// A snapshot over exactly what's committed so far, at `sample_rate_hz` and with no markers
    /// — enough for [`crate::peaks_query::peaks`] to read the pieces committed up to this instant
    /// (a plain clone of the current piece list: cheap next to a chunk commit, and this is called
    /// at most once per `import_peaks_get`, not per bucket).
    pub fn snapshot(&self, sample_rate_hz: u32) -> DocSnapshot {
        let audio = self.0.lock().unwrap_or_else(|e| e.into_inner());
        DocSnapshot::new(sample_rate_hz, audio.pieces.clone(), Vec::new())
    }
}

/// Appends samples to the store, committing a chunk every 65 536 samples and the remainder on
/// [`ChunkWriter::finish`]. Owned by one worker or capture-writer thread.
///
/// Cancelling (or dropping) a writer commits nothing to the document: chunks it already wrote
/// are unreachable store data, reclaimed by compaction (T-301). After an error, [`Self::committed`]
/// describes what is durably in the store; the writer should then be dropped.
pub struct ChunkWriter {
    store: Arc<ChunkStore>,
    buf: Vec<f32>,
    audio: WrittenAudio,
    progress: WriterProgress,
    cancel: CancelToken,
    /// H-71: set by [`Self::track_live`] — mirrors every commit so another thread can read the
    /// growing piece table (`None` costs nothing beyond the check on every commit for every other
    /// writer, e.g. recording/edit write-backs, that never calls `track_live`).
    live: Option<Arc<Mutex<WrittenAudio>>>,
}

impl ChunkWriter {
    /// A writer with its own cancel token.
    pub fn new(store: Arc<ChunkStore>) -> Self {
        Self::with_cancel(store, CancelToken::new())
    }

    /// A writer that stops with [`ProjectError::Cancelled`] once `cancel` is cancelled.
    pub fn with_cancel(store: Arc<ChunkStore>, cancel: CancelToken) -> Self {
        ChunkWriter {
            store,
            buf: Vec::with_capacity(CHUNK_SAMPLES),
            audio: WrittenAudio::default(),
            progress: WriterProgress::default(),
            cancel,
            live: None,
        }
    }

    /// Appends samples; every full chunk is committed immediately (peaks, CRC, flush).
    pub fn append(&mut self, samples: &[f32]) -> Result<()> {
        if self.cancel.is_cancelled() {
            return Err(ProjectError::Cancelled);
        }
        let mut rest = samples;
        while !rest.is_empty() {
            let n = (CHUNK_SAMPLES - self.buf.len()).min(rest.len());
            self.buf.extend_from_slice(&rest[..n]);
            rest = &rest[n..];
            self.progress.0.fetch_add(n as u64, Ordering::Relaxed);
            if self.buf.len() == CHUNK_SAMPLES {
                self.commit_buffer()?;
            }
        }
        Ok(())
    }

    fn commit_buffer(&mut self) -> Result<()> {
        if self.buf.is_empty() {
            return Ok(());
        }
        let loc = self.store.commit_chunk(&self.buf)?;
        record_commit(&mut self.audio, loc.id, loc.len);
        if let Some(live) = &self.live {
            let mut guard = live.lock().unwrap_or_else(|e| e.into_inner());
            record_commit(&mut guard, loc.id, loc.len);
        }
        self.buf.clear();
        Ok(())
    }

    /// Samples accepted so far (committed + pending).
    pub fn samples_written(&self) -> u64 {
        self.audio.len_samples + self.buf.len() as u64
    }

    /// What has been committed to the store so far.
    pub fn committed(&self) -> &WrittenAudio {
        &self.audio
    }

    /// A handle to observe progress from another thread.
    pub fn progress(&self) -> WriterProgress {
        self.progress.clone()
    }

    /// H-71 (SPEC-005 §2.3, ADR-003 Amendment 6): starts (or returns the existing) live handle —
    /// a clone tracking every chunk this writer commits from here on, readable from another
    /// thread via [`LiveWrittenAudio`]. Call once, right after construction and before the first
    /// `append`, so the handle never misses a commit.
    pub fn track_live(&mut self) -> LiveWrittenAudio {
        let live = self
            .live
            .get_or_insert_with(|| Arc::new(Mutex::new(self.audio.clone())));
        LiveWrittenAudio(Arc::clone(live))
    }

    /// The token that cancels this writer.
    pub fn cancel_token(&self) -> CancelToken {
        self.cancel.clone()
    }

    /// Commits the pending partial chunk and returns everything written.
    pub fn finish(mut self) -> Result<WrittenAudio> {
        if self.cancel.is_cancelled() {
            return Err(ProjectError::Cancelled);
        }
        self.commit_buffer()?;
        Ok(std::mem::take(&mut self.audio))
    }

    /// Like [`Self::finish`], but on failure still returns what was committed before the error
    /// ("finalize at the last good sample", ADR-004 §7) together with the error.
    pub fn finish_lossy(mut self) -> (WrittenAudio, Option<ProjectError>) {
        let error = self.commit_buffer().err();
        (std::mem::take(&mut self.audio), error)
    }

    /// Abandons the writer; nothing becomes part of any document.
    pub fn cancel(self) {}
}

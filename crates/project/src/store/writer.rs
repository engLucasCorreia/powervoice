//! [`ChunkWriter`]: streams new audio into the store as immutable chunks (ADR-004 §3).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use crate::snapshot::{Piece, push_piece};
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
        push_piece(&mut self.audio.pieces, Piece::chunk(loc.id, 0, loc.len));
        self.audio.chunks.push(loc.id);
        self.audio.len_samples += u64::from(loc.len);
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

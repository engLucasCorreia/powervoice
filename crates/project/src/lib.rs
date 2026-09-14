//! Document model (ADR-004): the disk-backed session store behind the one open document.
//!
//! - [`session`]: the session directory (`<sessions>/<id>/`), its advisory lock, `meta.json`, and
//!   the control-thread API that ties the store, the journal and the undo history together.
//! - [`store`]: the append-only chunk store (65 536-sample immutable chunks in preallocated 64 MiB
//!   segments, written through `memmap2`, CRC32 per chunk, per-chunk peak pyramids), the
//!   [`ChunkWriter`] and the memory budget with LRU segment eviction.
//! - [`snapshot`]: [`DocSnapshot`] (piece table + markers + `rev`/`audio_rev`), piece splicing and
//!   the marker-shifting rules.
//! - [`reader`]: [`SnapshotReader`], which copies `[pos, pos + n)` of a snapshot into a buffer.
//! - [`take`]: the crash-safe 32-bit float take WAV writer and WAV-tail recovery.
//! - [`journal`]: the append-only, CRC-checked, `fdatasync`ed edit journal.
//! - [`history`]: [`Edit`]s and the undo/redo stacks of snapshots.
//! - [`gc`]: start-up classification and deletion of session directories.
//!
//! # Concurrency contract
//! - **Control thread** owns the [`Session`] (`&mut self` API). Every command that changes the
//!   document appends its journal record and `fdatasync`s it before it returns `Ok`.
//! - **Reader thread** (playback prefetch) owns one [`SnapshotReader`] per stream. It shares the
//!   `Arc<ChunkStore>` and holds an `Arc<DocSnapshot>`, which no writer ever mutates. A read takes
//!   two short internal locks, each held only for a few pointer operations, or one `mmap` call when
//!   a segment has to be (re)mapped: the chunk-index `RwLock` (writers hold it only to push one
//!   entry) and the segment-table `Mutex`. Unmapping evicted segments happens after that mutex is
//!   released. Readers never touch the allocation lock that writers hold while preallocating a new
//!   segment. A read can still block on page faults (disk I/O) and verifies each chunk's CRC32 the
//!   first time the chunk is read, so it must never run on an audio thread (ADR-002).
//! - **Writer threads** (capture-writer, workers) each own a [`ChunkWriter`] or a
//!   [`take::TakeWriter`]. Chunk slots are reserved under a lock, so several writers may append
//!   concurrently. A chunk becomes readable only after its bytes are in the mapping and flushed.
//! - Snapshots are immutable and shared by `Arc`; dropping the last reference to one is cheap and
//!   never touches the store.

pub mod disk;
pub mod edit;
pub mod error;
mod fs_util;
pub mod gc;
pub mod history;
pub mod import;
pub mod journal;
pub mod normalize;
pub mod peaks_query;
pub mod reader;
pub mod save;
pub mod session;
pub mod snapshot;
pub mod store;
pub mod take;
pub mod vxpk;

pub use disk::{FixedFreeSpace, FreeSpaceProvider, SystemFreeSpace};
pub use edit::{PostEdit, Range, RangeError, Target as EditTarget, validate_range};
pub use error::{ProjectError, Result};
pub use history::{Edit, EditOp, History, HistoryStep, MarkerMapping, MarkerOp};
pub use import::{
    CHANNEL_PROBE_WINDOW_S, ImportChannel, ImportProbe, ImportResult, MAX_CHANNELS,
    MAX_SAMPLE_RATE_HZ, MIN_SAMPLE_RATE_HZ, import_file, import_wav, probe_for_import,
};
pub use normalize::{
    ALREADY_TOL_DB, LABEL_NORMALIZE, LABEL_NORMALIZE_LUFS, LufsNormalizeOutcome,
    LufsNormalizeResult, MIN_INTEGRATED_LUFS, MIN_PEAK_DBFS, NormalizeLufsPlan, NormalizeOutcome,
    NormalizePeakPlan, NormalizeResult, TRUE_PEAK_WARN_CEILING_DBTP,
    applied_post_edit as normalize_applied_post_edit, normalize_lufs, normalize_peak,
    plan_normalize_lufs, plan_normalize_peak, scan_loudness, scan_peak_accelerated,
    target_db_to_pct, target_linear, target_pct_to_db,
};
pub use peaks_query::{PEAKS_RAW_SPP, peaks};
pub use reader::SnapshotReader;
pub use save::save_snapshot_wav;
pub use session::{
    CloseError, FinishedTake, Session, SessionConfig, SourceInfo, TAKE_LABEL_KEY, TakeCapture,
    TakeId, TakeMode,
};
pub use snapshot::{DocSnapshot, Marker, MarkerId, Piece, Source};
pub use store::{
    CancelToken, ChunkId, ChunkLocation, ChunkPeaks, ChunkStore, ChunkWriter, MemoryReservation,
    StoreOptions, WriterProgress, WrittenAudio,
};
pub use take::{TakeWriter, TakeWriterOptions};
pub use vxpk::{VXPK_HEADER_LEN, VXPK_MAGIC, VXPK_VERSION, VxpkHeader, encode_vxpk};

/// Samples per chunk (ADR-004 §2, SPEC-000 `CHUNK_SAMPLES`).
pub const CHUNK_SAMPLES: usize = 65_536;
/// Bytes per full chunk (f32 samples).
pub const CHUNK_BYTES: usize = CHUNK_SAMPLES * 4;
/// Size of one preallocated, separately mapped store segment (ADR-004 §2).
pub const SEGMENT_BYTES: u64 = 64 * 1024 * 1024;
/// Chunks start at multiples of this inside a segment (ADR-004 §2).
pub const CHUNK_ALIGN_BYTES: u64 = 4096;
/// Version of the session store layout written to `meta.json`.
pub const STORE_FORMAT_VERSION: u32 = 1;

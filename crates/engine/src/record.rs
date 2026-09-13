//! Recording (S1-04, SPEC-002 §2.1–§2.3, §2.7 Off/Dry): the public types of the record API.
//!
//! Flow (the app owns the [`vox_project::Session`], the engine owns streams and threads):
//! 1. [`crate::EngineHandle::set_armed`] opens the input stream (the input meter starts). Its
//!    [`RecordState::input_rate_hz`] is the rate the new document must use (the input opens at
//!    the preferred record rate when the device supports it, else at its default rate).
//! 2. The app creates/clears the session, calls `Session::begin_take` and hands the
//!    [`vox_project::TakeCapture`] to [`crate::EngineHandle::record_start`] together with a
//!    [`RecordDone`] callback.
//! 3. The input callback pushes the take (trimmed to capture times `[t_record, t_stop)`) into the
//!    10 s capture ring; the capture-writer thread drains it at least every 50 ms into the take
//!    (WAV + chunk store); a sync thread patches + `fdatasync`s the WAV about every second.
//! 4. [`crate::EngineHandle::record_stop`] (or input-device loss) ends the take; the writer calls
//!    `TakeCapture::finish()` and then the [`RecordDone`] callback **on the writer thread** (which
//!    may call the [`crate::EngineHandle`]): the app checks [`RecordingResult::finished`]'s
//!    `error`, calls `Session::commit_take`, and publishes the new snapshot with
//!    `EngineHandle::set_document`.

use vox_project::{FinishedTake, ProjectError};

use crate::device_state::DeviceStatus;

/// SPEC-002 §3 `disk_floor_mib`: recording stops when free space on the session volume falls
/// below this (AC-13).
pub const DISK_FLOOR_BYTES: u64 = 512 * 1024 * 1024;
/// SPEC-002 §2.5: recording writes two copies (the take WAV and the session chunks) — 8 bytes/s
/// per Hz of the document rate (4-byte f32 samples × 2 copies).
pub const RECORD_BYTES_PER_SAMPLE: u64 = 8;
/// SPEC-002 §3 `disk_warn_min`: below this many minutes of remaining recording time, Record
/// confirms first ("Only N min of disk space left. Record anyway?").
pub const DISK_WARN_MINUTES: u64 = 10;
/// SPEC-002 §3 `dropout_fill_max_s` / §2.4 second row (A-011): input gaps longer than this are
/// device loss (stop, keep the take up to the gap), not a fillable dropout.
pub const DROPOUT_FILL_MAX_S: u64 = 2;

/// Monitoring mode (SPEC-002 §2.7). Through-rack monitoring is T-107 (not in this slice).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MonitorMode {
    /// No monitoring (factory default).
    #[default]
    Off,
    /// The input is added to the output after the rack, at unity gain, while armed or recording.
    Dry,
}

/// The record panel's state (`EngineEvent::Record`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecordState {
    /// The configured input device (`None` = "None": Arm and Record are unavailable).
    pub input_device: Option<String>,
    /// Applied 1-based input channel.
    pub input_channel: u16,
    /// Input status dot.
    pub input_status: DeviceStatus,
    /// The user armed the input (never persisted, SPEC-002 §2.1).
    pub armed: bool,
    /// The input stream is open (meter running).
    pub input_open: bool,
    /// Rate of the open input stream = the rate a new recording must use.
    pub input_rate_hz: Option<u32>,
    /// A take is being captured.
    pub recording: bool,
    /// Stop was requested; the capture-writer is finishing and committing the take.
    pub finishing: bool,
    /// Monitoring mode (preference).
    pub monitor: MonitorMode,
    /// Monitoring is audible now (armed or recording, mode ≠ off, output open).
    pub monitoring: bool,
    /// H-10 item 4: dropout events so far this take (SPEC-002 §2.1/§2.2's live amber counter),
    /// `0` while not recording. Read from [`crate::input::InputShared::dropout_events`] each
    /// tick — the same count [`RecordingResult::dropouts`] reports at Stop.
    pub dropout_count: u32,
    /// H-11 (SPEC-002 §2.5, AC-13): remaining recording time on the session volume at the
    /// current (or, while idle/armed, the configured default) rate — `(free − 512 MiB) / (8 ×
    /// rate)`, floored to whole seconds so it settles once a second instead of every tick.
    /// `None` when the free-space query failed (e.g. the volume doesn't exist yet).
    pub disk_remaining_s: Option<u64>,
}

/// Why a take ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StopReason {
    /// Record/Stop/Space.
    User,
    /// The input device was lost: the take is finalized at the last good sample (SPEC-002 §2.6).
    InputLost,
    /// The engine shut down while recording.
    Shutdown,
    /// The capture ring overflowed (the writer fell 10 s behind, e.g. a disk stall): the take
    /// ends at the last sample that fit (SPEC-002 §2.4, AC-8).
    Overflow,
    /// Appending to the take failed ([`RecordingResult::write_error`]): the recording stopped
    /// and the take is kept up to the last good sample (SPEC-002 §2.5).
    WriteError,
    /// H-11 (SPEC-002 §2.5, AC-13): free space on the session volume fell below
    /// [`DISK_FLOOR_BYTES`]. The take is finalized at the last good sample and kept, like
    /// [`StopReason::WriteError`].
    DiskFull,
}

/// What the capture-writer hands to [`RecordDone`] once the take is finished.
#[derive(Debug)]
pub struct RecordingResult {
    /// The finished take (check its `error`, then `Session::commit_take`).
    pub finished: FinishedTake,
    /// Why it ended.
    pub reason: StopReason,
    /// The take's sample rate (= the document rate it was started with; the capture-writer
    /// resamples when the input stream ran at a different rate, H-06).
    pub sample_rate_hz: u32,
    /// Clip events during the take (runs < 10 ms apart count once, SPEC-002 §2.1).
    pub clip_events: u32,
    /// Samples that didn't fit in the full capture ring (> 0 only with
    /// [`StopReason::Overflow`]: the take ended just before them).
    pub overflow_samples: u64,
    /// First failure appending to the take while recording ([`StopReason::WriteError`]: the
    /// recording stopped, and the rest of the take was dropped).
    pub write_error: Option<ProjectError>,
    /// H-10 item 4 (SPEC-002 §2.4/§4.3, AC-7): dropouts filled with silence during the take, in
    /// take order — the caller turns each into a "Dropout N ms" marker alongside the take's own
    /// undo entry. H-11 item 5: spliced and marked on the resampled capture path too (H-06), in
    /// document-rate samples like every other position here — `pos_samples` is read from the
    /// take's own (already document-rate) sample count, so resampling needs no extra mapping.
    pub dropouts: Vec<DropoutMark>,
}

/// One dropout the capture-writer filled with silence (H-10 item 4), in take-relative samples at
/// the take's rate ([`RecordingResult::sample_rate_hz`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DropoutMark {
    /// Where the gap starts (samples already written to the take before it).
    pub pos_samples: u64,
    /// How many silent samples were inserted.
    pub len_samples: u64,
}

/// Called once, on the capture-writer thread (or inline in a `ManualEngine`), when a take is
/// finished. May call the `EngineHandle`.
pub type RecordDone = Box<dyn FnOnce(RecordingResult) + Send>;

/// Bucket size for [`crate::EngineHandle::live_take_peaks`] (H-07): fixed, and small enough for a
/// responsive waveform while recording. Computed only on the capture-writer thread as it drains
/// the capture ring — never in the RT input callback (CLAUDE.md real-time rules). It matches one
/// of `vox_project`'s pyramid levels, so the UI can reduce it into pixel columns the same way.
pub const LIVE_PEAKS_SPB: u32 = 256;

/// A snapshot of the take being captured's running min/max peaks (H-07,
/// [`crate::EngineHandle::live_take_peaks`]). `buckets[0]` covers take samples
/// `[start_bucket * spb, (start_bucket + 1) * spb)`; the last bucket may be partial (covers only
/// the samples appended to it so far).
#[derive(Clone, Debug, PartialEq)]
pub struct LiveTakePeaks {
    /// The take's sample rate.
    pub sample_rate_hz: u32,
    /// Samples appended to the take so far.
    pub len_samples: u64,
    /// Samples per bucket ([`LIVE_PEAKS_SPB`]).
    pub spb: u32,
    /// `(min, max)` per bucket, starting at the requested `start_bucket`.
    pub buckets: Vec<(f32, f32)>,
}

/// Why `record_start` was refused. The take the caller began is **not** used: discard it
/// (`Session::discard_take`).
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum RecordError {
    /// A take is being recorded or finished.
    #[error("a recording is already running")]
    AlreadyRecording,
    /// No input device is configured.
    #[error("no input device selected")]
    NoInputDevice,
    /// The input stream could not be opened.
    #[error("the input stream is not open")]
    InputNotOpen,
    /// The capture-writer's resampler couldn't be built for the input and document rates (H-06:
    /// mismatched rates are otherwise resampled on the capture-writer thread, never rejected —
    /// this is a construction failure, e.g. an unsupported rate).
    #[error(
        "the input runs at {input_hz} Hz, the document at {doc_hz} Hz, and no resampler could be built"
    )]
    RateMismatch {
        /// Input stream rate.
        input_hz: u32,
        /// Requested document rate.
        doc_hz: u32,
    },
    /// The capture-writer thread could not be started.
    #[error("could not start the capture writer: {0}")]
    Writer(String),
    /// The engine is not running.
    #[error("the audio engine is not running")]
    EngineStopped,
}

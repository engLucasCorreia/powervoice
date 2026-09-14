//! Recording (S1-04, SPEC-002 §2.1–§2.3) and monitoring (§2.7, T-107): the public types of the
//! record API.
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

/// Monitoring mode (SPEC-002 §2.7). Audible only while armed or recording; switching fades over
/// ≤ 10 ms; never recorded (the take is always the dry input).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MonitorMode {
    /// No monitoring (factory default).
    #[default]
    Off,
    /// The input is added to the output after the rack, at unity gain, while armed or recording.
    Dry,
    /// T-107: the input is added to the rack input, together with any playback (one shared rack,
    /// ADR-002 §4), so the talent hears the processed voice.
    ThroughRack,
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
    /// T-107 (SPEC-002 §2.7, §4.4): the monitoring latency readout in µs (multiples of 100):
    /// input latency + monitor buffer target F* + rack latency (through-rack only) + output
    /// latency. `None` while the mode is Off, a stream is closed or not yet measured. Recomputed
    /// at least every 0.5 s.
    pub monitor_latency_us: Option<u32>,
    /// T-107: monitor underruns (fade out/in; SPEC-002 §2.7's monitor-dropout counter) since the
    /// output stream opened. The take is unaffected (no marker).
    pub monitor_underruns: u32,
    /// T-107: monitor overruns (dropped back to F* with a crossfade) since the output opened.
    pub monitor_overruns: u32,
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
    /// T-304 (SPEC-022): how a record operation (Insert / Overwrite / Punch) ended — `None` for a
    /// plain new recording (SPEC-002), whose whole take is the document audio.
    pub op: Option<OpResult>,
}

/// A record operation's phase (SPEC-022 §2.1, §4.9 `record_phase`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecordPhase {
    /// Playback before the record point.
    PreRoll,
    /// The record window: the part that enters the document.
    Recording,
    /// Playback after a punch.
    PostRoll,
    /// The operation ended; the take is being finished and committed.
    Committing,
}

/// `EngineEvent::RecordPhase`'s payload (SPEC-022 §4.9): sent at each phase change.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RecordPhaseInfo {
    /// The take (`vox_project::TakeId.0`).
    pub take: u32,
    pub kind: crate::record_op::RecordOpKind,
    pub phase: RecordPhase,
    /// Document position where the phase starts (the pre-roll start clamps at 0).
    pub doc_pos_samples: u64,
    /// App-clock time of the phase change (heard time of `doc_pos_samples` where known).
    pub app_ns: u64,
}

/// Why a record operation was cancelled (SPEC-022 §2.10): no edit, the document is unchanged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CancelReason {
    /// Stop / Space during pre-roll, or a stop at or before the record point.
    User,
    /// The input device was lost before the window opened.
    InputLost,
    /// The output device was lost during pre-roll: the talent can't hear the lead-in.
    OutputLost,
    /// The take ended (ring overflow, disk floor, write error) before the window opened.
    Aborted,
}

/// A record operation's result (`RecordingResult::op`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OpResult {
    pub plan: crate::record_op::RecordPlan,
    /// The record window `[k_start, k_end)` in take samples (document rate); `k_end` may exceed
    /// the take (the commit clips it to what was captured). `None` when cancelled.
    pub window: Option<(u64, u64)>,
    /// Set when the operation was cancelled.
    pub cancelled: Option<CancelReason>,
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
    /// T-304 (SPEC-022 §2.2): a punch-in needs an output device for pre-roll and post-roll.
    #[error("punch-in needs an output device")]
    PunchNeedsOutput,
    /// T-304: the operation doesn't fit the current document (stale position or length).
    #[error("the record position is outside the document")]
    InvalidPosition,
    /// T-304 (SPEC-022 §2.14): a calibration run is in progress.
    #[error("a latency calibration is running")]
    Calibrating,
}

/// Why a calibration run could not start or complete (SPEC-022 §2.14).
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum CalibrationError {
    /// Refused while a record operation runs.
    #[error("not available while recording")]
    Recording,
    /// Another calibration is running.
    #[error("a calibration is already running")]
    Busy,
    /// Calibration needs an output device.
    #[error("no output device")]
    NoOutput,
    /// Calibration needs an input device.
    #[error("no input device")]
    NoInput,
    /// Input and output must run at the same rate.
    #[error("the input runs at {input_hz} Hz and the output at {output_hz} Hz")]
    RateMismatch { input_hz: u32, output_hz: u32 },
    /// A device was lost or the run timed out.
    #[error("the calibration was interrupted")]
    Interrupted,
    /// Cancelled by the user.
    #[error("cancelled")]
    Cancelled,
}

/// A finished calibration capture (SPEC-022 §4.7), analysed off the control thread with
/// `vox_dsp::calibration::analyze`.
#[derive(Clone, Debug, PartialEq)]
pub struct CalibrationCapture {
    /// Device rate of both streams.
    pub rate_hz: u32,
    /// The sweep that was played.
    pub sweep: Vec<f32>,
    /// The input captured with automatic alignment.
    pub recording: Vec<f32>,
    /// Recording index aligned (heard time + δ) to each repetition's first sample.
    pub rep_starts: Vec<i64>,
    /// The offset δ that was applied (a Verify pass), ns.
    pub offset_ns: i64,
}

/// [`crate::EngineHandle::calibration_poll`]'s answer.
#[derive(Clone, Debug, PartialEq)]
pub enum CalibrationStatus {
    /// No run.
    Idle,
    /// Running; `progress` in `[0, 1]`.
    Running { progress: f32 },
    /// Finished (taken once).
    Done(Result<CalibrationCapture, CalibrationError>),
}

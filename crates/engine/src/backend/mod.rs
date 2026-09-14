//! Audio backend abstraction (ADR-002 §1, §7; SPEC-001 §4).
//!
//! One [`Backend`] trait with two implementations:
//! - [`cpal::CpalBackend`] (feature `backend-cpal`): real devices through cpal 0.18.
//! - [`fake::FakeBackend`]: deterministic, scripted devices for tests (always available).
//!
//! Devices are keyed by a stable `(host, name)` pair ([`DeviceKey`]), never by index. Input and
//! output are separate streams, each with its own clock: timestamps are nanoseconds on *that
//! stream's* clock and must never be compared across streams (ADR-002 §8, MEMORY gotcha).
//!
//! Real-time contract for everything called from a stream callback: no allocation, locks, I/O,
//! logging or panics. Backend error callbacks only set bits in [`StreamStatus`].
//!
//! Callback contract (T-105/T-106 rely on it):
//! - One device callback = one call of the [`InputCallback`]/[`OutputCallback`], with the app
//!   clock read exactly once ([`InputTimestamp::now_ns`]). Callback code must use that value and
//!   never read the clock again. Only if a non-f32 device delivers more frames than the stream's
//!   conversion buffer (sized from the opened buffer size) is a block split into consecutive calls
//!   sharing `now_ns`/`callback_ns`, with advancing capture/playback times.
//! - Presence and capabilities are separate ([`CapsState`]): a device whose capabilities cannot be
//!   read is still present.

#[cfg(feature = "backend-cpal")]
pub mod cpal;
pub mod fake;

use std::fmt;
use std::str::FromStr;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Instant;

use serde::{Deserialize, Serialize};

/// An audio host (backend API). Serialized as the lowercase ids of SPEC-001 §3.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HostId {
    /// Linux ALSA (always compiled in on Linux).
    Alsa,
    /// Linux PipeWire (cpal feature `pipewire`).
    #[serde(rename = "pipewire")]
    PipeWire,
    /// JACK (cpal feature `jack`).
    Jack,
    /// Windows WASAPI.
    Wasapi,
    /// macOS CoreAudio.
    #[serde(rename = "coreaudio")]
    CoreAudio,
}

impl HostId {
    /// Every host id, in the order used for listing.
    pub const ALL: [HostId; 5] = [
        HostId::PipeWire,
        HostId::Jack,
        HostId::Alsa,
        HostId::Wasapi,
        HostId::CoreAudio,
    ];

    /// The lowercase id (`"alsa"`, `"pipewire"`, …), as persisted.
    pub fn as_str(self) -> &'static str {
        match self {
            HostId::Alsa => "alsa",
            HostId::PipeWire => "pipewire",
            HostId::Jack => "jack",
            HostId::Wasapi => "wasapi",
            HostId::CoreAudio => "coreaudio",
        }
    }
}

impl fmt::Display for HostId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for HostId {
    type Err = BackendError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        HostId::ALL
            .into_iter()
            .find(|h| h.as_str().eq_ignore_ascii_case(s))
            .ok_or_else(|| BackendError::UnknownHost(s.to_owned()))
    }
}

/// The default host among the available ones (SPEC-001 §2.1, §3): Linux → PipeWire when
/// available, else ALSA; Windows → WASAPI; macOS → CoreAudio; otherwise the first available.
pub fn choose_default_host(available: &[HostId]) -> Option<HostId> {
    [
        HostId::PipeWire,
        HostId::Alsa,
        HostId::Wasapi,
        HostId::CoreAudio,
    ]
    .into_iter()
    .find(|h| available.contains(h))
    .or_else(|| available.first().copied())
}

/// Stream direction.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Direction {
    /// Capture.
    Input,
    /// Playback.
    Output,
}

/// Stable device identity: host plus the host-reported name (unique within that host's
/// enumeration). Persisted choices use this, never an index (SPEC-001 §2.2).
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct DeviceKey {
    /// The host the device belongs to.
    pub host: HostId,
    /// Host-reported device name, unique within the host.
    pub name: String,
}

impl DeviceKey {
    /// Builds a key.
    pub fn new(host: HostId, name: impl Into<String>) -> Self {
        Self {
            host,
            name: name.into(),
        }
    }
}

impl fmt::Display for DeviceKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.host, self.name)
    }
}

/// An inclusive range of supported sample rates.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RateRange {
    /// Lowest supported rate.
    pub min_hz: u32,
    /// Highest supported rate.
    pub max_hz: u32,
}

impl RateRange {
    /// A single discrete rate.
    pub const fn exactly(hz: u32) -> Self {
        Self {
            min_hz: hz,
            max_hz: hz,
        }
    }

    /// True if `hz` lies in the range.
    pub fn contains(&self, hz: u32) -> bool {
        self.min_hz <= hz && hz <= self.max_hz
    }
}

/// An inclusive range of supported buffer sizes (frames per callback).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BufferRange {
    /// Smallest buffer size.
    pub min_frames: u32,
    /// Largest buffer size.
    pub max_frames: u32,
}

/// Capabilities of one direction of a device.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DirectionCaps {
    /// Maximum channel count (input streams open all of them and deinterleave, SPEC-001 §4).
    pub max_channels: u16,
    /// Channel count of the device's default configuration.
    pub default_channels: u16,
    /// Supported sample rates, sorted and non-overlapping.
    pub rates: Vec<RateRange>,
    /// The device's default sample rate.
    pub default_rate_hz: u32,
    /// Supported buffer sizes; `None` when the host cannot report them.
    pub buffer_range: Option<BufferRange>,
}

impl DirectionCaps {
    /// True if the device can run at `hz`.
    pub fn supports_rate(&self, hz: u32) -> bool {
        self.rates.iter().any(|r| r.contains(hz))
    }

    /// Sorts and merges `rates` (overlapping or adjacent ranges become one).
    pub fn normalize_rates(rates: &mut Vec<RateRange>) {
        rates.retain(|r| r.min_hz <= r.max_hz && r.max_hz > 0);
        rates.sort_by_key(|r| (r.min_hz, r.max_hz));
        let mut merged: Vec<RateRange> = Vec::with_capacity(rates.len());
        for r in rates.drain(..) {
            match merged.last_mut() {
                Some(last) if r.min_hz <= last.max_hz.saturating_add(1) => {
                    last.max_hz = last.max_hz.max(r.max_hz);
                }
                _ => merged.push(r),
            }
        }
        *rates = merged;
    }
}

/// What is known about one supported direction of a present device.
///
/// Presence and capabilities are separate: a device whose capabilities cannot be read (busy,
/// held exclusively, some ALSA entries) is still present, so it is handled by the busy/lost rules
/// of SPEC-001 §2.2 (the open attempt decides), never as "not found — using default".
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CapsState {
    /// Read successfully.
    Known(DirectionCaps),
    /// Not read yet (first phase of an enumeration; a `changed` diff follows).
    Pending,
    /// Reading failed.
    Unavailable,
}

impl CapsState {
    /// The capabilities, if known.
    pub fn known(&self) -> Option<&DirectionCaps> {
        match self {
            CapsState::Known(c) => Some(c),
            _ => None,
        }
    }
}

/// One enumerated device.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceInfo {
    /// Unique (within the host) device name; the key's `name`: the host-reported name, with the
    /// backend id appended where needed for uniqueness (always on ALSA).
    pub name: String,
    /// The host-reported (display) name.
    pub base_name: String,
    /// Stable backend id (cpal device id: PipeWire node name, ALSA PCM id, …).
    pub id: String,
    /// Capture side; `None` = the device cannot capture.
    pub input: Option<CapsState>,
    /// Playback side; `None` = the device cannot play.
    pub output: Option<CapsState>,
    /// A "follow the system default" pseudo-device (PipeWire `input_default`, `output_default`,
    /// `sink_default`; ALSA `default`).
    pub system_default: bool,
    /// The capture side records another device's output (a PipeWire sink monitor), not a
    /// microphone or line input.
    pub input_is_monitor: bool,
}

impl DeviceInfo {
    /// A device named `name` (base name and id = `name`) with no directions and no flags.
    pub fn new(name: impl Into<String>) -> Self {
        let name = name.into();
        Self {
            base_name: name.clone(),
            id: name.clone(),
            name,
            input: None,
            output: None,
            system_default: false,
            input_is_monitor: false,
        }
    }

    /// Sets the capture side.
    pub fn with_input(mut self, caps: CapsState) -> Self {
        self.input = Some(caps);
        self
    }

    /// Sets the playback side.
    pub fn with_output(mut self, caps: CapsState) -> Self {
        self.output = Some(caps);
        self
    }

    /// True if the device supports `dir` (whatever its capability state).
    pub fn supports(&self, dir: Direction) -> bool {
        self.caps_state(dir).is_some()
    }

    /// Capability state for `dir` (`None` = direction unsupported).
    pub fn caps_state(&self, dir: Direction) -> Option<&CapsState> {
        match dir {
            Direction::Input => self.input.as_ref(),
            Direction::Output => self.output.as_ref(),
        }
    }

    /// Known capabilities for `dir`.
    pub fn caps(&self, dir: Direction) -> Option<&DirectionCaps> {
        self.caps_state(dir).and_then(CapsState::known)
    }

    /// How well `saved` names this device: 0 exact, 1 `"<base> (<id>)"`, 2 base name.
    fn match_rank(&self, saved: &str) -> Option<u8> {
        if self.name == saved {
            return Some(0);
        }
        let with_id = saved
            .strip_prefix(self.base_name.as_str())
            .and_then(|r| r.strip_prefix(" ("))
            .and_then(|r| r.strip_suffix(')'));
        if with_id == Some(self.id.as_str()) {
            return Some(1);
        }
        (self.base_name == saved).then_some(2)
    }
}

/// How much capability reading an enumeration may do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Enumerate {
    /// Names and directions only: capabilities from earlier passes, else `Pending`. Never opens a
    /// device (fast first phase of a poll).
    Quick,
    /// Reads the capabilities of devices not read before (background poll; querying can mean
    /// opening the device, which fails while we stream on it).
    Cached,
    /// Reads every device's capabilities again (manual rescan). A failed read keeps what was
    /// known before.
    Fresh,
}

/// Requested buffer size for a stream.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BufferRequest {
    /// The device/host default ("Auto", the factory default).
    #[default]
    Auto,
    /// A fixed number of frames per callback.
    Frames(u32),
}

/// Parameters for opening one stream.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StreamRequest {
    /// The device.
    pub device: DeviceKey,
    /// Sample rate; must be supported by the device.
    pub sample_rate_hz: u32,
    /// Buffer size request.
    pub buffer: BufferRequest,
    /// Channel count; `None` = all input channels (input) or the default count (output).
    pub channels: Option<u16>,
}

/// What an open stream actually runs with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StreamInfo {
    /// Backend-assigned id, unique per backend instance.
    pub id: StreamId,
    /// The device.
    pub device: DeviceKey,
    /// Direction.
    pub direction: Direction,
    /// Interleaved channels delivered to / expected from the callback.
    pub channels: u16,
    /// Sample rate.
    pub sample_rate_hz: u32,
    /// The buffer request that was applied.
    pub buffer: BufferRequest,
    /// The backend's estimate of frames per callback, if known. Callbacks may deliver any size.
    pub nominal_frames: Option<u32>,
    /// Whether this input stream's capture timestamps can be trusted to measure a dropout's
    /// length (SPEC-002 §4.3, ADR-002 §7). `false` falls back to the callback-gap rule and marks
    /// dropouts "length unknown" with no silence fill (H-23). Always `true` for output streams;
    /// real backends report `true` until T-105/T-106 add per-backend detection (PipeWire/JACK).
    pub timestamps_reliable: bool,
}

/// Backend-assigned stream id.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct StreamId(pub u64);

/// Timestamps of one input callback. `callback_ns`/`capture_ns` are on this stream's own clock;
/// `now_ns` is the app clock (ADR-002 §8) read once when the callback started. Map a stream
/// instant to the app clock with [`to_app_ns`](Self::to_app_ns).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InputTimestamp {
    /// App clock (ns since [`app_epoch`]; simulated time under the fake backend) at the callback.
    pub now_ns: u64,
    /// When the callback was invoked (stream clock).
    pub callback_ns: u64,
    /// When the first frame of the buffer was captured (stream clock). `callback − capture` is
    /// at least the buffer duration plus the device's input latency.
    pub capture_ns: u64,
}

impl InputTimestamp {
    /// App-clock time of stream instant `stream_ns`: `now + (x − callback)`.
    pub fn to_app_ns(&self, stream_ns: u64) -> u64 {
        stream_to_app(self.now_ns, self.callback_ns, stream_ns)
    }
}

/// Timestamps of one output callback; see [`InputTimestamp`] for the clocks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OutputTimestamp {
    /// App clock at the callback.
    pub now_ns: u64,
    /// When the callback was invoked (stream clock).
    pub callback_ns: u64,
    /// When the first frame of the buffer will be heard (stream clock). `playback − callback` =
    /// output latency.
    pub playback_ns: u64,
}

impl OutputTimestamp {
    /// App-clock time of stream instant `stream_ns`: `now + (x − callback)`.
    pub fn to_app_ns(&self, stream_ns: u64) -> u64 {
        stream_to_app(self.now_ns, self.callback_ns, stream_ns)
    }
}

#[inline]
fn stream_to_app(now_ns: u64, callback_ns: u64, x: u64) -> u64 {
    (i128::from(now_ns) + i128::from(x) - i128::from(callback_ns)).max(0) as u64
}

static APP_EPOCH: OnceLock<Instant> = OnceLock::new();

/// The process-wide app-clock origin (ADR-002 §8 `APP_EPOCH`); fixed on first use.
pub fn app_epoch() -> Instant {
    *APP_EPOCH.get_or_init(Instant::now)
}

/// The app clock now: ns since [`app_epoch`]. The cpal wrappers call it exactly once per
/// callback (the one permitted time read, ADR-002 §2) and pass it as `now_ns`.
#[inline]
pub fn app_now_ns() -> u64 {
    app_epoch().elapsed().as_nanos() as u64
}

/// Nanoseconds spanned by `frames` at `rate_hz` (0 if the rate is 0).
#[inline]
pub fn frames_to_ns(frames: u64, rate_hz: u32) -> u64 {
    if rate_hz == 0 {
        return 0;
    }
    ((u128::from(frames) * 1_000_000_000) / u128::from(rate_hz)) as u64
}

/// Real-time input callback. `data` is interleaved with `channels` channels per frame.
/// Must follow the RT contract (no allocation, locks, I/O, logging, panics) and use
/// `ts.now_ns` instead of reading the clock (see the module docs for the once-per-callback rule).
pub trait InputCallback: Send + 'static {
    /// Called for every captured buffer.
    fn process(&mut self, data: &[f32], channels: usize, ts: InputTimestamp);
}

impl<F> InputCallback for F
where
    F: FnMut(&[f32], usize, InputTimestamp) + Send + 'static,
{
    fn process(&mut self, data: &[f32], channels: usize, ts: InputTimestamp) {
        self(data, channels, ts)
    }
}

/// Real-time output callback. Fill `data` (interleaved, `channels` per frame; pre-zeroed).
/// Must follow the RT contract.
pub trait OutputCallback: Send + 'static {
    /// Called for every buffer to be played.
    fn process(&mut self, data: &mut [f32], channels: usize, ts: OutputTimestamp);
}

impl<F> OutputCallback for F
where
    F: FnMut(&mut [f32], usize, OutputTimestamp) + Send + 'static,
{
    fn process(&mut self, data: &mut [f32], channels: usize, ts: OutputTimestamp) {
        self(data, channels, ts)
    }
}

/// Stream error flag bits (ADR-002 §7).
pub mod flags {
    /// The device disappeared or the stream can no longer run.
    pub const DEVICE_LOST: u32 = 1 << 0;
    /// A backend error that did not (yet) end the stream.
    pub const BACKEND_ERROR: u32 = 1 << 1;
}

/// Classification of a backend error report, used by the error-callback path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorClass {
    /// The device/stream is gone → [`flags::DEVICE_LOST`].
    DeviceLost,
    /// An xrun reported by the backend (counted only).
    Xrun,
    /// Informational (e.g. rerouted default device); counted only.
    Benign,
    /// Anything else → [`flags::BACKEND_ERROR`].
    Other,
}

/// Lock-free per-stream status shared by the stream callbacks and the control thread.
/// Every method is RT-safe (atomics only).
#[derive(Debug, Default)]
pub struct StreamStatus {
    flags: AtomicU32,
    errors: AtomicU32,
    xruns: AtomicU32,
    callbacks: AtomicU64,
}

impl StreamStatus {
    /// New status with no flags.
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Sets flag bits (RT-safe).
    #[inline]
    pub fn raise(&self, bits: u32) {
        self.flags.fetch_or(bits, Ordering::Release);
    }

    /// Records an error report from the backend's error callback (RT-safe).
    #[inline]
    pub fn report(&self, class: ErrorClass) {
        match class {
            ErrorClass::DeviceLost => {
                self.errors.fetch_add(1, Ordering::Relaxed);
                self.raise(flags::DEVICE_LOST);
            }
            ErrorClass::Other => {
                self.errors.fetch_add(1, Ordering::Relaxed);
                self.raise(flags::BACKEND_ERROR);
            }
            ErrorClass::Xrun => {
                self.xruns.fetch_add(1, Ordering::Relaxed);
            }
            ErrorClass::Benign => {}
        }
    }

    /// Counts one data callback (RT-safe).
    #[inline]
    pub fn count_callback(&self) {
        self.callbacks.fetch_add(1, Ordering::Relaxed);
    }

    /// Current flag bits without clearing them.
    pub fn peek(&self) -> u32 {
        self.flags.load(Ordering::Acquire)
    }

    /// Returns and clears the flag bits (control thread, once per tick).
    pub fn take(&self) -> u32 {
        self.flags.swap(0, Ordering::AcqRel)
    }

    /// True if [`flags::DEVICE_LOST`] is set.
    pub fn is_lost(&self) -> bool {
        self.peek() & flags::DEVICE_LOST != 0
    }

    /// Number of error reports (lost + other).
    pub fn error_count(&self) -> u32 {
        self.errors.load(Ordering::Relaxed)
    }

    /// Number of backend-reported xruns.
    pub fn xrun_count(&self) -> u32 {
        self.xruns.load(Ordering::Relaxed)
    }

    /// Number of data callbacks so far.
    pub fn callback_count(&self) -> u64 {
        self.callbacks.load(Ordering::Relaxed)
    }
}

/// Minimum callback silence before a running stream counts as silently dead.
pub const STALL_MIN_NS: u64 = 500_000_000;

/// Period assumed when a stream's callback size is unknown.
const STALL_UNKNOWN_PERIOD_FRAMES: u32 = 4096;

/// Detects a stream that stopped calling back without reporting an error (silent death).
///
/// The control thread's tick (T-105) must call [`check`](Self::check) for every open stream with
/// its [`StreamStatus::callback_count`] and the app clock; `true` means "handle as
/// [`flags::DEVICE_LOST`]". Threshold: max(500 ms, 4 periods).
#[derive(Clone, Debug)]
pub struct StallDetector {
    threshold_ns: u64,
    last_count: u64,
    last_progress_ns: u64,
}

impl StallDetector {
    /// For a stream just opened at app time `now_ns` (period from its nominal callback size).
    pub fn new(info: &StreamInfo, now_ns: u64) -> Self {
        let frames = info.nominal_frames.unwrap_or(STALL_UNKNOWN_PERIOD_FRAMES);
        Self::with_period(frames_to_ns(u64::from(frames), info.sample_rate_hz), now_ns)
    }

    /// For a stream with callback period `period_ns`.
    pub fn with_period(period_ns: u64, now_ns: u64) -> Self {
        Self {
            threshold_ns: STALL_MIN_NS.max(period_ns.saturating_mul(4)),
            last_count: 0,
            last_progress_ns: now_ns,
        }
    }

    /// Silence that counts as a stall.
    pub fn threshold_ns(&self) -> u64 {
        self.threshold_ns
    }

    /// Feeds the current callback count; `true` once no callback arrived for the threshold.
    pub fn check(&mut self, callback_count: u64, now_ns: u64) -> bool {
        if callback_count != self.last_count {
            self.last_count = callback_count;
            self.last_progress_ns = now_ns;
            return false;
        }
        now_ns.saturating_sub(self.last_progress_ns) >= self.threshold_ns
    }
}

/// Keeps a backend stream alive; dropping it closes the stream.
pub trait StreamGuard: Send {}

impl<T: Send> StreamGuard for T {}

/// An open stream. Dropping the handle stops and closes it. Drop it on the control thread, never
/// on an audio thread.
///
/// **Where the callback is dropped.** cpal owns the callback closure — and everything it owns,
/// e.g. T-105's rack — on its worker thread and drops it there, inside the join that
/// `Stream::drop` performs before this handle's drop returns. A callback owning heavy state must
/// therefore not tear it down in its own drop (e.g. `Module::deactivate` must stay on the control
/// thread, ADR-005): its `Drop` impl should hand the state back into a slot (e.g. an
/// `Arc<Mutex<Option<…>>>`) that the control thread takes after dropping the handle. The fake
/// backend drops callbacks on the thread that drops the handle, outside its lock.
pub struct StreamHandle {
    info: StreamInfo,
    status: Arc<StreamStatus>,
    _guard: Box<dyn StreamGuard>,
}

impl StreamHandle {
    /// Wraps a backend stream guard.
    pub fn new(info: StreamInfo, status: Arc<StreamStatus>, guard: Box<dyn StreamGuard>) -> Self {
        Self {
            info,
            status,
            _guard: guard,
        }
    }

    /// What the stream runs with.
    pub fn info(&self) -> &StreamInfo {
        &self.info
    }

    /// The stream's lock-free status (flags, counters).
    pub fn status(&self) -> &Arc<StreamStatus> {
        &self.status
    }
}

impl fmt::Debug for StreamHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StreamHandle")
            .field("info", &self.info)
            .field("flags", &self.status.peek())
            .finish()
    }
}

/// Backend errors (non-RT paths only).
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum BackendError {
    /// A host id string that is not one of SPEC-001's hosts.
    #[error("unknown audio host \"{0}\"")]
    UnknownHost(String),
    /// The host is not compiled in or not reachable.
    #[error("audio host {0} is not available")]
    HostUnavailable(HostId),
    /// The device is not (or no longer) reported by its host.
    #[error("audio device {0} not found")]
    DeviceNotFound(DeviceKey),
    /// The device exists but not in the requested direction.
    #[error("audio device {0} does not support {1:?}")]
    WrongDirection(DeviceKey, Direction),
    /// Rate/channels/buffer/format not supported.
    #[error("unsupported stream configuration: {0}")]
    UnsupportedConfig(String),
    /// The device is in use by someone else.
    #[error("audio device {0} is busy")]
    DeviceBusy(DeviceKey),
    /// Any other backend failure.
    #[error("audio backend error: {0}")]
    Backend(String),
}

/// Result of enumerating one host.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceSnapshot {
    /// Devices in host order, names unique.
    pub devices: Vec<DeviceInfo>,
    /// Name of the host's default input device, if any.
    pub default_input: Option<String>,
    /// Name of the host's default output device, if any.
    pub default_output: Option<String>,
}

impl DeviceSnapshot {
    /// Removes duplicate names (keeping the first) and defaults that name absent devices.
    pub fn normalize(&mut self) {
        let mut seen = std::collections::HashSet::new();
        self.devices.retain(|d| seen.insert(d.name.clone()));
        let dir_ok = |name: &Option<String>, dir: Direction, devices: &[DeviceInfo]| {
            name.as_ref()
                .is_some_and(|n| devices.iter().any(|d| &d.name == n && d.supports(dir)))
        };
        if !dir_ok(&self.default_input, Direction::Input, &self.devices) {
            self.default_input = None;
        }
        if !dir_ok(&self.default_output, Direction::Output, &self.devices) {
            self.default_output = None;
        }
    }

    /// The device named exactly `name`.
    pub fn get(&self, name: &str) -> Option<&DeviceInfo> {
        self.devices.iter().find(|d| d.name == name)
    }

    /// The device a saved or configured name refers to, among devices supporting `dir`: an exact
    /// name first, then `"<base> (<id>)"` (saved while it needed its id suffix), then the base
    /// name (saved before an identical second device appeared; the first in host order wins).
    /// Presence only — its capabilities may still be pending or unavailable.
    pub fn lookup(&self, saved: &str, dir: Direction) -> Option<&DeviceInfo> {
        self.devices
            .iter()
            .enumerate()
            .filter(|(_, d)| d.supports(dir))
            .filter_map(|(i, d)| d.match_rank(saved).map(|r| (r, i, d)))
            .min_by_key(|(r, i, _)| (*r, *i))
            .map(|(_, _, d)| d)
    }

    /// True if `saved` refers to a present device supporting `dir` (see [`lookup`](Self::lookup)).
    pub fn is_present(&self, saved: &str, dir: Direction) -> bool {
        self.lookup(saved, dir).is_some()
    }

    /// Known capabilities of the device `saved` refers to in `dir`.
    pub fn caps(&self, saved: &str, dir: Direction) -> Option<&DirectionCaps> {
        self.lookup(saved, dir).and_then(|d| d.caps(dir))
    }

    /// Devices supporting `dir`, in host order.
    pub fn devices_for(&self, dir: Direction) -> impl Iterator<Item = &DeviceInfo> {
        self.devices.iter().filter(move |d| d.supports(dir))
    }

    /// True if some device's capabilities have not been read yet.
    pub fn has_pending(&self) -> bool {
        self.devices.iter().any(|d| {
            [&d.input, &d.output]
                .iter()
                .any(|c| matches!(c, Some(CapsState::Pending)))
        })
    }

    /// The host default device name for `dir`.
    pub fn default_for(&self, dir: Direction) -> Option<&str> {
        match dir {
            Direction::Input => self.default_input.as_deref(),
            Direction::Output => self.default_output.as_deref(),
        }
    }
}

/// An audio backend: host/device enumeration and stream creation.
///
/// Enumeration may block (ALSA can take hundreds of ms): call it from the device-poll or control
/// thread, never from the UI or an audio thread.
pub trait Backend: Send + Sync {
    /// Hosts that are compiled in and reachable right now, in listing order. May block.
    fn hosts(&self) -> Vec<HostId>;

    /// The default host (see [`choose_default_host`]).
    fn default_host(&self) -> Option<HostId> {
        choose_default_host(&self.hosts())
    }

    /// Enumerates the devices of `host`. The result is normalized (unique names).
    fn enumerate(&self, host: HostId, mode: Enumerate) -> Result<DeviceSnapshot, BackendError>;

    /// Opens and starts an input stream. The callback receives all device channels interleaved.
    fn open_input(
        &self,
        request: &StreamRequest,
        callback: Box<dyn InputCallback>,
    ) -> Result<StreamHandle, BackendError>;

    /// Opens and starts an output stream.
    fn open_output(
        &self,
        request: &StreamRequest,
        callback: Box<dyn OutputCallback>,
    ) -> Result<StreamHandle, BackendError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_host_prefers_pipewire_then_alsa() {
        use HostId::*;
        assert_eq!(choose_default_host(&[Jack, Alsa, PipeWire]), Some(PipeWire));
        assert_eq!(choose_default_host(&[Jack, Alsa]), Some(Alsa));
        assert_eq!(choose_default_host(&[Jack]), Some(Jack));
        assert_eq!(choose_default_host(&[Wasapi]), Some(Wasapi));
        assert_eq!(choose_default_host(&[CoreAudio]), Some(CoreAudio));
        assert_eq!(choose_default_host(&[]), None);
    }

    #[test]
    fn host_id_strings_round_trip() {
        for h in HostId::ALL {
            assert_eq!(h.as_str().parse::<HostId>().unwrap(), h);
            let json = serde_json::to_string(&h).unwrap();
            assert_eq!(json, format!("\"{}\"", h.as_str()));
            assert_eq!(serde_json::from_str::<HostId>(&json).unwrap(), h);
        }
        assert!("PipeWire".parse::<HostId>().is_ok());
        assert!("pulse".parse::<HostId>().is_err());
    }

    #[test]
    fn rates_normalize_merges_and_sorts() {
        let mut r = vec![
            RateRange::exactly(96_000),
            RateRange {
                min_hz: 8_000,
                max_hz: 48_000,
            },
            RateRange::exactly(44_100),
            RateRange::exactly(48_001),
        ];
        DirectionCaps::normalize_rates(&mut r);
        assert_eq!(
            r,
            vec![
                RateRange {
                    min_hz: 8_000,
                    max_hz: 48_001
                },
                RateRange::exactly(96_000)
            ]
        );
    }

    #[test]
    fn status_flags_and_counters() {
        let s = StreamStatus::new();
        s.report(ErrorClass::Xrun);
        s.report(ErrorClass::Benign);
        assert_eq!(s.peek(), 0);
        assert_eq!(s.xrun_count(), 1);
        s.report(ErrorClass::Other);
        assert_eq!(s.peek(), flags::BACKEND_ERROR);
        s.report(ErrorClass::DeviceLost);
        assert!(s.is_lost());
        assert_eq!(s.take(), flags::BACKEND_ERROR | flags::DEVICE_LOST);
        assert_eq!(s.peek(), 0);
        assert_eq!(s.error_count(), 2);
    }

    #[test]
    fn lookup_matches_exact_then_suffixed_then_base_name() {
        let mic = |name: &str, id: &str| DeviceInfo {
            base_name: "USB Mic".into(),
            id: id.into(),
            ..DeviceInfo::new(name).with_input(CapsState::Pending)
        };
        // Alone, the mic has its plain name.
        let alone = DeviceSnapshot {
            devices: vec![mic("USB Mic", "alsa_input.usb-A")],
            ..DeviceSnapshot::default()
        };
        let found = |s: &DeviceSnapshot, saved: &str| {
            s.lookup(saved, Direction::Input).map(|d| d.name.clone())
        };
        assert_eq!(found(&alone, "USB Mic").as_deref(), Some("USB Mic"));
        assert_eq!(
            found(&alone, "USB Mic (alsa_input.usb-A)").as_deref(),
            Some("USB Mic")
        );
        assert_eq!(found(&alone, "USB Mic (alsa_input.usb-B)"), None);
        assert!(alone.lookup("USB Mic", Direction::Output).is_none());
        // A second identical mic appears: both get suffixed names; the saved plain name still
        // resolves (no false loss), and a suffixed name picks its own device.
        let twins = DeviceSnapshot {
            devices: vec![
                mic("USB Mic (alsa_input.usb-A)", "alsa_input.usb-A"),
                mic("USB Mic (alsa_input.usb-B)", "alsa_input.usb-B"),
            ],
            ..DeviceSnapshot::default()
        };
        assert!(twins.is_present("USB Mic", Direction::Input));
        assert_eq!(
            found(&twins, "USB Mic (alsa_input.usb-B)").as_deref(),
            Some("USB Mic (alsa_input.usb-B)")
        );
        assert!(twins.has_pending());
    }

    #[test]
    fn stall_detector_threshold_and_progress() {
        let mut s = StallDetector::with_period(frames_to_ns(256, 48_000), 0);
        assert_eq!(s.threshold_ns(), STALL_MIN_NS);
        assert!(!s.check(0, 499_999_999));
        assert!(s.check(0, 500_000_000));
        assert!(!s.check(1, 600_000_000), "progress resets");
        assert!(!s.check(1, 1_099_999_999));
        assert!(s.check(1, 1_100_000_000));
        // Long periods: 4 × 8192 frames at 44.1 kHz ≈ 743 ms.
        let long = StallDetector::with_period(frames_to_ns(8192, 44_100), 0);
        assert_eq!(long.threshold_ns(), 4 * frames_to_ns(8192, 44_100));
    }

    #[test]
    fn app_clock_mapping() {
        let ts = InputTimestamp {
            now_ns: 1_000,
            callback_ns: 50_000,
            capture_ns: 49_400,
        };
        assert_eq!(ts.to_app_ns(ts.capture_ns), 400);
        assert_eq!(ts.to_app_ns(0), 0, "clamped at the epoch");
        let a = app_now_ns();
        assert!(app_now_ns() >= a);
    }

    #[test]
    fn frames_to_ns_is_exact_for_whole_seconds() {
        assert_eq!(frames_to_ns(48_000, 48_000), 1_000_000_000);
        assert_eq!(frames_to_ns(480, 48_000), 10_000_000);
        assert_eq!(frames_to_ns(1, 0), 0);
    }
}

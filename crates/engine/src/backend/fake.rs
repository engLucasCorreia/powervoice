//! Deterministic fake audio backend (ADR-002 §2, SPEC-000 glossary "Fake backend").
//!
//! Drives the *real* callback code with synthetic timing and injected faults, without hardware
//! or threads: time only moves when the test calls [`FakeBackend::advance_to`] /
//! [`advance_by`](FakeBackend::advance_by) (or a [`FakeDriver`] does, in real time). Everything
//! is reproducible from the seed.
//!
//! # Scripting
//! - Hosts with availability; devices with per-direction capabilities ([`FakeDirection`]).
//! - Hot-plug: [`plug`](FakeBackend::plug) / [`unplug`](FakeBackend::unplug), immediately or at
//!   a simulated time ([`schedule`](FakeBackend::schedule)).
//! - Faults per stream: loss ([`lose_stream`](FakeBackend::lose_stream)), non-fatal errors,
//!   input dropouts (frames skipped, capture timestamps jump), open failures per device.
//! - Callback sizes: the requested buffer size, a fixed size, or seeded random sizes
//!   (default range 1…4096).
//! - Input signals per channel ([`Signal`] or any closure); output recording.
//! - [`set_rt_guard`](FakeBackend::set_rt_guard) wraps every callback invocation (tests pass
//!   `vox_module_api::test_util::no_alloc`); violations are summed in
//!   [`rt_violations`](FakeBackend::rt_violations).
//!
//! # Timing model
//! Simulated time `t` is in ns since the backend was created and is the fake's **app clock**:
//! every callback gets it as `now_ns`. Each stream's clock is `t` plus a random per-stream
//! offset, so stream timestamps of different streams are not comparable (as with cpal); map them
//! with `to_app_ns`. A device runs at `rate × (1 + skew_ppm·10⁻⁶)` true frames per second; frame
//! `k` of a stream is at `origin + k / true_rate`.
//! - Input block of `N` frames starting at frame `F`: `capture = time(F)`; the callback comes once
//!   the block is complete, `callback = time(F + N) + latency (+ jitter)`. So
//!   `callback − capture = N / rate + latency`, as on real devices, and capture times are
//!   continuous (a gap means a dropout, SPEC-002 §4.3).
//! - Output block starting at frame `G`: `playback = time(G)`,
//!   `callback = playback − latency (+ jitter)`.
//! - Optional seeded jitter delays callbacks by `0..=jitter_ns` (never capture/playback times).
//!
//! Callbacks are invoked in callback-time order (ties: by stream id), outside the backend's lock,
//! so a control thread may open/close streams concurrently. Callbacks must not call into the
//! `FakeBackend` themselves.

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, Weak};
use std::thread::JoinHandle;
use std::time::Duration;

use super::{
    Backend, BackendError, BufferRange, BufferRequest, CapsState, DeviceInfo, DeviceKey,
    DeviceSnapshot, Direction, DirectionCaps, Enumerate, HostId, InputCallback, InputTimestamp,
    OutputCallback, OutputTimestamp, RateRange, StreamHandle, StreamId, StreamInfo, StreamRequest,
    StreamStatus, flags,
};

/// Largest random callback size (SPEC-000 AC-2: 1…4096).
pub const MAX_RANDOM_FRAMES: u32 = 4096;

/// Wraps one callback invocation; returns the number of RT violations it detected.
pub type RtGuard = Arc<dyn Fn(&mut dyn FnMut()) -> u32 + Send + Sync>;

/// Input sample source: `(frame index since stream start, channel, stream rate) -> sample`.
pub type SignalFn = Box<dyn FnMut(u64, usize, u32) -> f32 + Send>;

/// How many frames each callback delivers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CallbackSizes {
    /// The requested fixed buffer size, or the device's default buffer for `Auto`.
    Requested,
    /// Always `n` frames.
    Fixed(u32),
    /// Uniform in `min..=max` (seeded).
    Random {
        /// Smallest size (≥ 1).
        min: u32,
        /// Largest size.
        max: u32,
    },
}

impl CallbackSizes {
    /// Random sizes over the full 1…4096 range.
    pub const FULL_RANDOM: Self = Self::Random {
        min: 1,
        max: MAX_RANDOM_FRAMES,
    };
}

/// Built-in per-channel input signals.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Signal {
    /// Digital silence.
    Silence,
    /// `amplitude · sin(2π f k / rate)`.
    Sine {
        /// Frequency.
        freq_hz: f64,
        /// Peak amplitude.
        amplitude: f32,
    },
    /// Constant value.
    Dc(f32),
    /// Uniform white noise in `[-amplitude, amplitude)`, a pure function of (seed, frame, channel).
    Noise {
        /// Seed.
        seed: u64,
        /// Peak amplitude.
        amplitude: f32,
    },
}

impl Signal {
    /// Sample at `frame` (channel `ch`) for a stream at `rate_hz`.
    pub fn sample(&self, frame: u64, ch: usize, rate_hz: u32) -> f32 {
        match *self {
            Signal::Silence => 0.0,
            Signal::Dc(v) => v,
            Signal::Sine { freq_hz, amplitude } => {
                let phase = std::f64::consts::TAU * freq_hz * frame as f64 / f64::from(rate_hz);
                (phase.sin() * f64::from(amplitude)) as f32
            }
            Signal::Noise { seed, amplitude } => {
                let z =
                    mix64(seed ^ frame.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ ((ch as u64) << 56));
                let unit = (z >> 11) as f64 * (1.0 / (1u64 << 53) as f64);
                ((unit * 2.0 - 1.0) * f64::from(amplitude)) as f32
            }
        }
    }
}

/// T-304 (SPEC-022 §4.8): a loopback from an output device's playback into an input device
/// (a cable, or a microphone at a speaker). The output device must record its output
/// ([`FakeDirection::record_output`]; don't [`FakeBackend::take_recorded_output`] it meanwhile).
///
/// An input frame whose true capture time is `t` carries the output sample truly heard at
/// `t − residual` — so with `residual > 0` the recording lands late by exactly what the reported
/// timestamps miss (the hidden, unreported latency) — times `gain`, plus an optional echo, plus
/// seeded white noise. `mute` ranges (true time) silence the looped signal (not the noise).
#[derive(Clone, Debug, PartialEq)]
pub struct Loopback {
    /// The output device whose playback is looped back.
    pub output: DeviceKey,
    /// The input device capturing it (every channel receives the loop).
    pub input: DeviceKey,
    /// Loop gain.
    pub gain_db: f64,
    /// Unreported residual latency in ns (may be negative).
    pub residual_ns: i64,
    /// Echo `(delay ns, gain dB)` added to the direct path.
    pub echo: Option<(u64, f64)>,
    /// Additive uniform white noise `(seed, RMS dBFS)`.
    pub noise: Option<(u64, f64)>,
    /// True-time ranges `[start, end)` (ns) where the loop is silent.
    pub mute: Vec<(u64, u64)>,
}

impl Loopback {
    /// A clean 0 dB loopback with no residual.
    pub fn new(output: DeviceKey, input: DeviceKey) -> Self {
        Self {
            output,
            input,
            gain_db: 0.0,
            residual_ns: 0,
            echo: None,
            noise: None,
            mute: Vec::new(),
        }
    }
}

/// One direction of a fake device: capabilities plus timing/fault behavior.
pub struct FakeDirection {
    /// Reported capabilities.
    pub caps: DirectionCaps,
    /// Frames per callback for `BufferRequest::Auto` with [`CallbackSizes::Requested`].
    pub default_buffer_frames: u32,
    /// Callback size policy.
    pub callback_sizes: CallbackSizes,
    /// Device latency. Input: `callback − capture − block duration`; output:
    /// `playback − callback`.
    pub latency_ns: u64,
    /// Seeded callback jitter: each callback is delayed by `0..=jitter_ns`.
    pub jitter_ns: u64,
    /// True rate deviation from nominal, in ppm (clock skew).
    pub skew_ppm: f64,
    /// Record what output streams play (channel 0 + block timing).
    pub record: bool,
    /// Capabilities cannot be read (busy/exclusive): enumerated as `Unavailable`, opens fail with
    /// `DeviceBusy`.
    pub unreadable: bool,
    source: Option<SignalFn>,
}

impl FakeDirection {
    /// `channels` channels, discrete `rates`, default rate, buffer range 16…4096 (default 256),
    /// callback sizes as requested, no latency, no skew, silent input.
    pub fn new(channels: u16, rates: &[u32], default_rate_hz: u32) -> Self {
        let mut r: Vec<RateRange> = rates.iter().map(|&hz| RateRange::exactly(hz)).collect();
        DirectionCaps::normalize_rates(&mut r);
        Self {
            caps: DirectionCaps {
                max_channels: channels,
                default_channels: channels,
                rates: r,
                default_rate_hz,
                buffer_range: Some(BufferRange {
                    min_frames: 16,
                    max_frames: MAX_RANDOM_FRAMES,
                }),
            },
            default_buffer_frames: 256,
            callback_sizes: CallbackSizes::Requested,
            latency_ns: 0,
            jitter_ns: 0,
            skew_ppm: 0.0,
            record: false,
            unreadable: false,
            source: None,
        }
    }

    /// Sets the seeded callback jitter (callbacks delayed by `0..=ns`).
    pub fn jitter_ns(mut self, ns: u64) -> Self {
        self.jitter_ns = ns;
        self
    }

    /// Makes the capabilities unreadable (device busy/held exclusively).
    pub fn unreadable(mut self) -> Self {
        self.unreadable = true;
        self
    }

    /// Sets the supported buffer range (`None` = unknown).
    pub fn buffer_range(mut self, range: Option<(u32, u32)>) -> Self {
        self.caps.buffer_range = range.map(|(min_frames, max_frames)| BufferRange {
            min_frames,
            max_frames,
        });
        self
    }

    /// Sets the frames per callback used for `Auto`.
    pub fn default_buffer(mut self, frames: u32) -> Self {
        self.default_buffer_frames = frames.max(1);
        self
    }

    /// Sets the callback size policy.
    pub fn callback_sizes(mut self, sizes: CallbackSizes) -> Self {
        self.callback_sizes = sizes;
        self
    }

    /// Sets the latency in nanoseconds.
    pub fn latency_ns(mut self, ns: u64) -> Self {
        self.latency_ns = ns;
        self
    }

    /// Sets the clock skew in ppm.
    pub fn skew_ppm(mut self, ppm: f64) -> Self {
        self.skew_ppm = ppm;
        self
    }

    /// Sets the default channel count (≤ max).
    pub fn default_channels(mut self, channels: u16) -> Self {
        self.caps.default_channels = channels.clamp(1, self.caps.max_channels.max(1));
        self
    }

    /// Records output streams on this device.
    pub fn record_output(mut self) -> Self {
        self.record = true;
        self
    }

    /// Input source: one [`Signal`] per channel (missing channels are silent).
    pub fn signals(self, signals: Vec<Signal>) -> Self {
        self.source(move |frame, ch, rate| {
            signals.get(ch).map_or(0.0, |s| s.sample(frame, ch, rate))
        })
    }

    /// Input source closure `(frame, channel, rate) -> sample`.
    pub fn source(mut self, f: impl FnMut(u64, usize, u32) -> f32 + Send + 'static) -> Self {
        self.source = Some(Box::new(f));
        self
    }
}

/// A scripted device.
pub struct FakeDevice {
    /// Unique name within its host (the key name).
    pub name: String,
    /// Host-reported base name (default: `name`).
    pub base_name: Option<String>,
    /// Stable backend id (default: `name`).
    pub id: Option<String>,
    /// Reported as a system-default pseudo-device.
    pub system_default: bool,
    /// Its capture side is a monitor of an output.
    pub input_is_monitor: bool,
    /// Capture side.
    pub input: Option<FakeDirection>,
    /// Playback side.
    pub output: Option<FakeDirection>,
}

impl FakeDevice {
    /// A device with no directions yet.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            base_name: None,
            id: None,
            system_default: false,
            input_is_monitor: false,
            input: None,
            output: None,
        }
    }

    /// Sets the host-reported base name and the stable id (e.g. one of two identical devices).
    pub fn identity(mut self, base_name: impl Into<String>, id: impl Into<String>) -> Self {
        self.base_name = Some(base_name.into());
        self.id = Some(id.into());
        self
    }

    /// Marks the device as a system-default pseudo-device.
    pub fn system_default(mut self) -> Self {
        self.system_default = true;
        self
    }

    /// Marks the capture side as a monitor of an output.
    pub fn monitor(mut self) -> Self {
        self.input_is_monitor = true;
        self
    }

    /// Adds a capture side.
    pub fn with_input(mut self, dir: FakeDirection) -> Self {
        self.input = Some(dir);
        self
    }

    /// Adds a playback side.
    pub fn with_output(mut self, dir: FakeDirection) -> Self {
        self.output = Some(dir);
        self
    }

    fn side(&self, dir: Direction) -> Option<&FakeDirection> {
        match dir {
            Direction::Input => self.input.as_ref(),
            Direction::Output => self.output.as_ref(),
        }
    }

    fn info(&self, pending: bool) -> DeviceInfo {
        let state = |d: &FakeDirection| {
            if pending {
                CapsState::Pending
            } else if d.unreadable {
                CapsState::Unavailable
            } else {
                CapsState::Known(d.caps.clone())
            }
        };
        DeviceInfo {
            name: self.name.clone(),
            base_name: self.base_name.clone().unwrap_or_else(|| self.name.clone()),
            id: self.id.clone().unwrap_or_else(|| self.name.clone()),
            input: self.input.as_ref().map(state),
            output: self.output.as_ref().map(state),
            system_default: self.system_default,
            input_is_monitor: self.input_is_monitor,
        }
    }
}

/// A scripted event, applied at a simulated time (see [`FakeBackend::schedule`]).
// Test scripting only (a handful per test); boxing the device would only complicate call sites.
#[allow(clippy::large_enum_variant)]
pub enum FakeEvent {
    /// Hot-plug a device (replaces a device of the same name).
    Plug(HostId, FakeDevice),
    /// Unplug a device: removed from enumeration, its streams raise `DEVICE_LOST` and stop.
    Unplug(DeviceKey),
    /// Loss of one stream (device stays listed): `DEVICE_LOST`, callbacks stop.
    LoseStream(StreamId),
    /// Raise flag bits on a stream; it keeps running unless `DEVICE_LOST` is among them.
    StreamError(StreamId, u32),
    /// Input dropout: skip `frames` device frames (capture timestamps jump). Ignored for outputs.
    InputDropout(StreamId, u64),
    /// Silent death: callbacks stop without any error flag (see `StallDetector`).
    Stall(StreamId),
    /// Host availability.
    HostAvailable(HostId, bool),
}

/// Output recorded by a fake output stream.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RecordedOutput {
    /// Channel 0 of every buffer, in order.
    pub samples: Vec<f32>,
    /// One entry per callback.
    pub blocks: Vec<RecordedBlock>,
}

/// Timing of one recorded output block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RecordedBlock {
    /// First frame index since stream start.
    pub first_frame: u64,
    /// Frames in the block.
    pub frames: u32,
    /// Playback timestamp (stream clock).
    pub playback_ns: u64,
}

/// Observable state of a fake stream.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FakeStreamState {
    /// Stream info as returned at open.
    pub info: StreamInfo,
    /// Still running (not lost, not closed).
    pub alive: bool,
    /// The handle was dropped.
    pub closed: bool,
    /// Callbacks so far.
    pub callbacks: u64,
    /// Frames delivered so far (dropped frames excluded).
    pub frames: u64,
    /// Next device frame index (dropped frames included).
    pub frame_pos: u64,
}

struct FakeHost {
    id: HostId,
    available: bool,
    devices: Vec<FakeDevice>,
}

enum Cb {
    Input(Option<Box<dyn InputCallback>>),
    Output(Option<Box<dyn OutputCallback>>),
}

struct Slot {
    info: StreamInfo,
    status: Arc<StreamStatus>,
    alive: bool,
    closed: bool,
    cb: Cb,
    buf: Vec<f32>,
    sizes: CallbackSizes,
    rng: Rng,
    true_rate: f64,
    latency_ns: u64,
    jitter_ns: u64,
    origin_ns: u64,
    clock_offset_ns: u64,
    frame_pos: u64,
    next_frames: u32,
    next_at: u64,
    callbacks: u64,
    frames: u64,
    record: Option<RecordedOutput>,
}

impl Slot {
    fn frame_time(&self, frame: u64) -> u64 {
        self.origin_ns + (frame as f64 * 1e9 / self.true_rate).round() as u64
    }

    /// Draws the next block size and schedules its callback.
    fn schedule_next(&mut self, now_ns: u64) {
        self.next_frames = match self.sizes {
            CallbackSizes::Fixed(n) => n.max(1),
            CallbackSizes::Random { min, max } => self.rng.range(min.max(1), max.max(1)),
            CallbackSizes::Requested => 256,
        };
        self.reschedule(now_ns);
    }

    /// Callback time of the block at `frame_pos` (`next_frames` long): an input block once it
    /// is complete plus latency, an output block `latency` before it plays; plus jitter.
    fn reschedule(&mut self, now_ns: u64) {
        let base = match self.info.direction {
            Direction::Input => {
                self.frame_time(self.frame_pos + u64::from(self.next_frames)) + self.latency_ns
            }
            Direction::Output => self
                .frame_time(self.frame_pos)
                .saturating_sub(self.latency_ns),
        };
        let jitter = if self.jitter_ns > 0 {
            self.rng.below(self.jitter_ns + 1)
        } else {
            0
        };
        self.next_at = (base + jitter).max(now_ns);
    }

    fn kill(&mut self, bits: u32) {
        self.status.raise(bits);
        self.alive = false;
    }
}

struct State {
    hosts: Vec<FakeHost>,
    now_ns: u64,
    seed: u64,
    rng: Rng,
    slots: Vec<Slot>,
    schedule: Vec<(u64, u64, FakeEvent)>,
    seq: u64,
    guard: Option<RtGuard>,
    rt_violations: u64,
    open_failures: Vec<(DeviceKey, BackendError)>,
    pending_caps: bool,
    /// Devices whose capabilities a `Cached`/`Fresh` enumeration has read (like cpal's cache).
    caps_read: HashSet<(HostId, String)>,
    /// T-304: the output → input loopback, if any.
    loopback: Option<Loopback>,
}

impl State {
    fn host_mut(&mut self, id: HostId) -> &mut FakeHost {
        if let Some(i) = self.hosts.iter().position(|h| h.id == id) {
            return &mut self.hosts[i];
        }
        self.hosts.push(FakeHost {
            id,
            available: true,
            devices: Vec::new(),
        });
        self.hosts.last_mut().expect("just pushed")
    }

    fn device(&self, key: &DeviceKey) -> Option<&FakeDevice> {
        self.hosts
            .iter()
            .find(|h| h.id == key.host && h.available)?
            .devices
            .iter()
            .find(|d| d.name == key.name)
    }

    fn device_mut(&mut self, key: &DeviceKey) -> Option<&mut FakeDevice> {
        self.hosts
            .iter_mut()
            .find(|h| h.id == key.host)?
            .devices
            .iter_mut()
            .find(|d| d.name == key.name)
    }

    fn slot_mut(&mut self, id: StreamId) -> Option<&mut Slot> {
        self.slots.iter_mut().find(|s| s.info.id == id)
    }

    fn apply(&mut self, ev: FakeEvent) -> Option<FakeDevice> {
        match ev {
            FakeEvent::Plug(host, dev) => {
                let h = self.host_mut(host);
                match h.devices.iter_mut().find(|d| d.name == dev.name) {
                    Some(existing) => *existing = dev,
                    None => h.devices.push(dev),
                }
                None
            }
            FakeEvent::Unplug(key) => {
                let h = self.hosts.iter_mut().find(|h| h.id == key.host)?;
                let i = h.devices.iter().position(|d| d.name == key.name)?;
                let dev = h.devices.remove(i);
                self.caps_read.remove(&(key.host, key.name.clone()));
                for s in self
                    .slots
                    .iter_mut()
                    .filter(|s| s.alive && s.info.device == key)
                {
                    s.kill(flags::DEVICE_LOST);
                }
                Some(dev)
            }
            FakeEvent::LoseStream(id) => {
                if let Some(s) = self.slot_mut(id).filter(|s| s.alive) {
                    s.kill(flags::DEVICE_LOST);
                }
                None
            }
            FakeEvent::StreamError(id, bits) => {
                if let Some(s) = self.slot_mut(id).filter(|s| s.alive) {
                    if bits & flags::DEVICE_LOST != 0 {
                        s.kill(bits);
                    } else {
                        s.status.raise(bits);
                    }
                }
                None
            }
            FakeEvent::InputDropout(id, frames) => {
                let now = self.now_ns;
                if let Some(s) = self
                    .slot_mut(id)
                    .filter(|s| s.info.direction == Direction::Input)
                {
                    s.frame_pos += frames;
                    s.reschedule(now);
                }
                None
            }
            FakeEvent::Stall(id) => {
                if let Some(s) = self.slot_mut(id) {
                    s.alive = false;
                }
                None
            }
            FakeEvent::HostAvailable(host, available) => {
                self.host_mut(host).available = available;
                if !available {
                    for s in self
                        .slots
                        .iter_mut()
                        .filter(|s| s.alive && s.info.device.host == host)
                    {
                        s.kill(flags::DEVICE_LOST);
                    }
                }
                None
            }
        }
    }
}

/// The fake backend. Cheap to clone (shared state).
#[derive(Clone)]
pub struct FakeBackend {
    state: Arc<Mutex<State>>,
}

impl std::fmt::Debug for FakeBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FakeBackend")
            .field("now_ns", &self.now_ns())
            .finish_non_exhaustive()
    }
}

fn lock(m: &Mutex<State>) -> MutexGuard<'_, State> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl FakeBackend {
    /// An empty backend (no hosts) seeded with `seed`.
    pub fn new(seed: u64) -> Self {
        Self {
            state: Arc::new(Mutex::new(State {
                hosts: Vec::new(),
                now_ns: 0,
                seed,
                rng: Rng(seed),
                slots: Vec::new(),
                schedule: Vec::new(),
                seq: 0,
                guard: None,
                rt_violations: 0,
                open_failures: Vec::new(),
                pending_caps: false,
                caps_read: HashSet::new(),
                loopback: None,
            })),
        }
    }

    /// T-304 (SPEC-022 §4.8): loops an output device's playback back into an input device
    /// (`None` removes it). See [`Loopback`].
    pub fn set_loopback(&self, loopback: Option<Loopback>) {
        self.lock().loopback = loopback;
    }

    /// Two-phase enumeration like the cpal backend on ALSA: [`Enumerate::Quick`] reports every
    /// capability as `Pending`, the other modes read them.
    pub fn set_pending_caps(&self, pending: bool) {
        self.lock().pending_caps = pending;
    }

    /// Stops a stream's callbacks without raising any flag (silent death) now.
    pub fn stall_stream(&self, id: StreamId) {
        self.lock().apply(FakeEvent::Stall(id));
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        lock(&self.state)
    }

    /// Adds (or updates) a host.
    pub fn add_host(&self, host: HostId, available: bool) -> &Self {
        self.lock().host_mut(host).available = available;
        self
    }

    /// Plugs `device` into `host` now (creating the host if needed). The first capable device of
    /// a host is its default for that direction.
    pub fn plug(&self, host: HostId, device: FakeDevice) -> &Self {
        self.lock().apply(FakeEvent::Plug(host, device));
        self
    }

    /// Unplugs a device now; returns it (e.g. to plug it back later).
    pub fn unplug(&self, key: &DeviceKey) -> Option<FakeDevice> {
        self.lock().apply(FakeEvent::Unplug(key.clone()))
    }

    /// Loses one stream now (`DEVICE_LOST`), the device stays listed.
    pub fn lose_stream(&self, id: StreamId) {
        self.lock().apply(FakeEvent::LoseStream(id));
    }

    /// Raises flag bits on a stream now.
    pub fn stream_error(&self, id: StreamId, bits: u32) {
        self.lock().apply(FakeEvent::StreamError(id, bits));
    }

    /// Skips `frames` input frames on a stream now.
    pub fn input_dropout(&self, id: StreamId, frames: u64) {
        self.lock().apply(FakeEvent::InputDropout(id, frames));
    }

    /// Sets host availability now (unavailable: its streams are lost).
    pub fn set_host_available(&self, host: HostId, available: bool) {
        self.lock().apply(FakeEvent::HostAvailable(host, available));
    }

    /// Makes every open of `key` fail with `error` (`None` clears).
    pub fn fail_opens(&self, key: &DeviceKey, error: Option<BackendError>) {
        let mut st = self.lock();
        st.open_failures.retain(|(k, _)| k != key);
        if let Some(e) = error {
            st.open_failures.push((key.clone(), e));
        }
    }

    /// Schedules `event` at simulated time `at_ns` (events at equal times apply in order).
    pub fn schedule(&self, at_ns: u64, event: FakeEvent) {
        let mut st = self.lock();
        st.seq += 1;
        let seq = st.seq;
        let at = at_ns.max(st.now_ns);
        let i = st
            .schedule
            .partition_point(|(t, s, _)| (*t, *s) <= (at, seq));
        st.schedule.insert(i, (at, seq, event));
    }

    /// Wraps every subsequent callback invocation in `guard` (see module docs).
    pub fn set_rt_guard(&self, guard: impl Fn(&mut dyn FnMut()) -> u32 + Send + Sync + 'static) {
        self.lock().guard = Some(Arc::new(guard));
    }

    /// Sum of violations reported by the RT guard.
    pub fn rt_violations(&self) -> u64 {
        self.lock().rt_violations
    }

    /// Current simulated time (ns).
    pub fn now_ns(&self) -> u64 {
        self.lock().now_ns
    }

    /// States of all streams ever opened, in open order.
    pub fn streams(&self) -> Vec<FakeStreamState> {
        self.lock()
            .slots
            .iter()
            .map(|s| FakeStreamState {
                info: s.info.clone(),
                alive: s.alive,
                closed: s.closed,
                callbacks: s.callbacks,
                frames: s.frames,
                frame_pos: s.frame_pos,
            })
            .collect()
    }

    /// The most recently opened live stream of `key` in `dir`.
    pub fn stream_for(&self, key: &DeviceKey, dir: Direction) -> Option<StreamId> {
        self.lock()
            .slots
            .iter()
            .rev()
            .find(|s| s.alive && &s.info.device == key && s.info.direction == dir)
            .map(|s| s.info.id)
    }

    /// What an output stream recorded (requires [`FakeDirection::record_output`]).
    pub fn recorded_output(&self, id: StreamId) -> Option<RecordedOutput> {
        self.lock()
            .slots
            .iter()
            .find(|s| s.info.id == id)
            .and_then(|s| s.record.clone())
    }

    /// T-107: moves out what an output stream recorded so far (recording continues), so long
    /// simulated runs can analyze the output in pieces instead of holding all of it.
    pub fn take_recorded_output(&self, id: StreamId) -> Option<RecordedOutput> {
        self.lock()
            .slots
            .iter_mut()
            .find(|s| s.info.id == id)
            .and_then(|s| s.record.as_mut().map(std::mem::take))
    }

    /// T-107: the simulated (app-clock) time of frame `frame` of stream `id` — an input frame's
    /// capture time or an output frame's playback time, at the device's true (skewed) rate.
    /// Dropped input frames count (see [`FakeEvent::InputDropout`]).
    pub fn frame_time_ns(&self, id: StreamId, frame: u64) -> Option<u64> {
        self.lock()
            .slots
            .iter()
            .find(|s| s.info.id == id)
            .map(|s| s.frame_time(frame))
    }

    /// Advances simulated time by `ns`, running callbacks and events due until then.
    pub fn advance_by(&self, ns: u64) {
        let t = self.now_ns().saturating_add(ns);
        self.advance_to(t);
    }

    /// Advances simulated time to `t_ns`, running every callback and event due at or before it.
    pub fn advance_to(&self, t_ns: u64) {
        loop {
            let mut st = self.lock();
            let ev_at = st.schedule.first().map(|e| e.0);
            let next_cb = st
                .slots
                .iter()
                .enumerate()
                .filter(|(_, s)| s.alive && !s.closed)
                .min_by_key(|(_, s)| (s.next_at, s.info.id))
                .map(|(i, s)| (i, s.next_at));
            match (ev_at, next_cb) {
                (Some(te), cb) if te <= t_ns && cb.is_none_or(|(_, tc)| te <= tc) => {
                    let (at, _, ev) = st.schedule.remove(0);
                    st.now_ns = st.now_ns.max(at);
                    st.apply(ev);
                }
                (_, Some((i, tc))) if tc <= t_ns => {
                    st.now_ns = st.now_ns.max(tc);
                    self.run_callback(st, i);
                }
                _ => {
                    st.now_ns = st.now_ns.max(t_ns);
                    return;
                }
            }
        }
    }

    /// Runs one callback of slot `i` (lock released during the user callback).
    fn run_callback(&self, mut st: MutexGuard<'_, State>, i: usize) {
        let (id, frames, channels) = {
            let s = &st.slots[i];
            (s.info.id, s.next_frames, usize::from(s.info.channels))
        };
        let n = frames as usize * channels;
        let dir = st.slots[i].info.direction;
        let rate = st.slots[i].info.sample_rate_hz;
        let first = st.slots[i].frame_pos;
        let frame_t = st.slots[i].frame_time(first);
        let off = st.slots[i].clock_offset_ns;
        let at = st.slots[i].next_at;

        // Prepare the buffer (outside the RT guard).
        let mut buf = std::mem::take(&mut st.slots[i].buf);
        buf.resize(n, 0.0);
        match dir {
            Direction::Input => {
                let key = st.slots[i].info.device.clone();
                let source = st
                    .device_mut(&key)
                    .and_then(|d| d.input.as_mut())
                    .and_then(|d| d.source.as_mut());
                match source {
                    Some(src) => {
                        for (f, frame) in buf.chunks_exact_mut(channels).enumerate() {
                            for (c, x) in frame.iter_mut().enumerate() {
                                *x = src(first + f as u64, c, rate);
                            }
                        }
                    }
                    None => buf.fill(0.0),
                }
                if let Some(lb) = st.loopback.as_ref().filter(|lb| lb.input == key) {
                    mix_loopback(&st, lb, i, first, channels, &mut buf);
                }
            }
            Direction::Output => buf.fill(0.0),
        }
        let mut cb = match &mut st.slots[i].cb {
            Cb::Input(c) => Cb::Input(c.take()),
            Cb::Output(c) => Cb::Output(c.take()),
        };
        let status = st.slots[i].status.clone();
        let guard = st.guard.clone();
        drop(st);

        let mut call = || {
            status.count_callback();
            match &mut cb {
                Cb::Input(Some(c)) => c.process(
                    &buf,
                    channels,
                    InputTimestamp {
                        now_ns: at,
                        callback_ns: at + off,
                        capture_ns: frame_t + off,
                    },
                ),
                Cb::Output(Some(c)) => c.process(
                    &mut buf,
                    channels,
                    OutputTimestamp {
                        now_ns: at,
                        callback_ns: at + off,
                        playback_ns: frame_t + off,
                    },
                ),
                _ => {}
            }
        };
        let violations = match &guard {
            Some(g) => g(&mut call),
            None => {
                call();
                0
            }
        };

        let mut st = self.lock();
        st.rt_violations += u64::from(violations);
        let now = st.now_ns;
        let Some(s) = st.slot_mut(id) else {
            drop(st);
            drop(cb); // stream gone: drop the callback outside the lock
            return;
        };
        if let Some(rec) = s.record.as_mut() {
            rec.samples.extend(buf.iter().step_by(channels.max(1)));
            rec.blocks.push(RecordedBlock {
                first_frame: first,
                frames,
                playback_ns: frame_t + off,
            });
        }
        if s.closed {
            drop(st);
            drop(cb); // closed while running: drop the callback outside the lock
            return;
        }
        s.cb = cb;
        s.buf = buf;
        s.callbacks += 1;
        s.frames += u64::from(frames);
        // A dropout injected during the callback has already moved `frame_pos` on; add to it.
        s.frame_pos += u64::from(frames);
        s.schedule_next(now);
    }

    fn open(
        &self,
        req: &StreamRequest,
        dir: Direction,
        cb: Cb,
    ) -> Result<StreamHandle, BackendError> {
        let mut st = self.lock();
        if let Some((_, e)) = st.open_failures.iter().find(|(k, _)| *k == req.device) {
            return Err(e.clone());
        }
        if !st
            .hosts
            .iter()
            .any(|h| h.id == req.device.host && h.available)
        {
            return Err(BackendError::HostUnavailable(req.device.host));
        }
        let dev = st
            .device(&req.device)
            .ok_or_else(|| BackendError::DeviceNotFound(req.device.clone()))?;
        let side = dev
            .side(dir)
            .ok_or_else(|| BackendError::WrongDirection(req.device.clone(), dir))?;
        if side.unreadable {
            return Err(BackendError::DeviceBusy(req.device.clone()));
        }
        let caps = &side.caps;
        if !caps.supports_rate(req.sample_rate_hz) {
            return Err(BackendError::UnsupportedConfig(format!(
                "{} Hz not supported by {}",
                req.sample_rate_hz, req.device
            )));
        }
        let channels = req.channels.unwrap_or(match dir {
            Direction::Input => caps.max_channels,
            Direction::Output => caps.default_channels,
        });
        if channels == 0 || channels > caps.max_channels {
            return Err(BackendError::UnsupportedConfig(format!(
                "{channels} channels not supported by {}",
                req.device
            )));
        }
        if let (BufferRequest::Frames(n), Some(r)) = (req.buffer, caps.buffer_range)
            && (n < r.min_frames || n > r.max_frames)
        {
            return Err(BackendError::UnsupportedConfig(format!(
                "buffer size {n} outside {}..={} of {}",
                r.min_frames, r.max_frames, req.device
            )));
        }
        let sizes = match (side.callback_sizes, req.buffer) {
            (CallbackSizes::Requested, BufferRequest::Frames(n)) => CallbackSizes::Fixed(n),
            (CallbackSizes::Requested, BufferRequest::Auto) => {
                CallbackSizes::Fixed(side.default_buffer_frames)
            }
            (other, _) => other,
        };
        let (max_frames, nominal) = match sizes {
            CallbackSizes::Fixed(n) => (n.max(1), Some(n.max(1))),
            CallbackSizes::Random { max, .. } => (max.max(1), None),
            CallbackSizes::Requested => (256, Some(256)),
        };
        let true_rate = f64::from(req.sample_rate_hz) * (1.0 + side.skew_ppm * 1e-6);
        let latency_ns = side.latency_ns;
        let jitter_ns = side.jitter_ns;
        let record = (dir == Direction::Output && side.record).then(RecordedOutput::default);

        st.seq += 1;
        let id = StreamId(st.seq);
        let clock_offset_ns = 1_000_000_000 + st.rng.below(1_000_000_000_000);
        let now = st.now_ns;
        let info = StreamInfo {
            id,
            device: req.device.clone(),
            direction: dir,
            channels,
            sample_rate_hz: req.sample_rate_hz,
            buffer: req.buffer,
            nominal_frames: nominal,
        };
        let status = StreamStatus::new();
        let mut slot = Slot {
            info: info.clone(),
            status: status.clone(),
            alive: true,
            closed: false,
            cb,
            buf: Vec::with_capacity(max_frames as usize * usize::from(channels)),
            sizes,
            rng: Rng(st.seed ^ id.0.wrapping_mul(0xD1B5_4A32_D192_ED03)),
            true_rate,
            latency_ns,
            jitter_ns,
            origin_ns: match dir {
                Direction::Input => now,
                Direction::Output => now + latency_ns,
            },
            clock_offset_ns,
            frame_pos: 0,
            next_frames: 0,
            next_at: 0,
            callbacks: 0,
            frames: 0,
            record,
        };
        slot.schedule_next(now);
        st.slots.push(slot);
        drop(st);
        let guard = FakeStreamGuard {
            state: Arc::downgrade(&self.state),
            id,
        };
        Ok(StreamHandle::new(info, status, Box::new(guard)))
    }

    /// Spawns a thread that advances simulated time in real time, `step` per `step`, until the
    /// returned driver is dropped. For tests that run a real control thread.
    pub fn spawn_driver(&self, step: Duration) -> FakeDriver {
        let stop = Arc::new(AtomicBool::new(false));
        let stop2 = stop.clone();
        let fake = self.clone();
        let step_ns = step.as_nanos() as u64;
        let join = std::thread::spawn(move || {
            while !stop2.load(Ordering::Relaxed) {
                std::thread::sleep(step);
                fake.advance_by(step_ns);
            }
        });
        FakeDriver {
            stop,
            join: Some(join),
        }
    }
}

/// Real-time driver thread handle; stops on drop.
pub struct FakeDriver {
    stop: Arc<AtomicBool>,
    join: Option<JoinHandle<()>>,
}

impl Drop for FakeDriver {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
    }
}

struct FakeStreamGuard {
    state: Weak<Mutex<State>>,
    id: StreamId,
}

impl Drop for FakeStreamGuard {
    fn drop(&mut self) {
        let Some(state) = self.state.upgrade() else {
            return;
        };
        let retired = {
            let mut st = lock(&state);
            st.slot_mut(self.id).map(|s| {
                s.closed = true;
                s.alive = false;
                let cb = match &mut s.cb {
                    Cb::Input(c) => Cb::Input(c.take()),
                    Cb::Output(c) => Cb::Output(c.take()),
                };
                (cb, std::mem::take(&mut s.buf))
            })
        };
        // The callback (and whatever it owns) is dropped here, outside the lock.
        drop(retired);
    }
}

impl Backend for FakeBackend {
    fn hosts(&self) -> Vec<HostId> {
        let st = self.lock();
        HostId::ALL
            .into_iter()
            .filter(|id| st.hosts.iter().any(|h| h.id == *id && h.available))
            .collect()
    }

    fn enumerate(&self, host: HostId, mode: Enumerate) -> Result<DeviceSnapshot, BackendError> {
        let mut guard = self.lock();
        let st = &mut *guard;
        let h = st
            .hosts
            .iter()
            .find(|h| h.id == host && h.available)
            .ok_or(BackendError::HostUnavailable(host))?;
        let devices: Vec<DeviceInfo> = h
            .devices
            .iter()
            .map(|d| {
                let unread = !st.caps_read.contains(&(host, d.name.clone()));
                d.info(st.pending_caps && mode == Enumerate::Quick && unread)
            })
            .collect();
        if mode != Enumerate::Quick {
            for d in &h.devices {
                st.caps_read.insert((host, d.name.clone()));
            }
        }
        let first = |dir| {
            devices
                .iter()
                .find(|d| d.supports(dir))
                .map(|d| d.name.clone())
        };
        let mut snap = DeviceSnapshot {
            default_input: first(Direction::Input),
            default_output: first(Direction::Output),
            devices,
        };
        snap.normalize();
        Ok(snap)
    }

    fn open_input(
        &self,
        request: &StreamRequest,
        callback: Box<dyn InputCallback>,
    ) -> Result<StreamHandle, BackendError> {
        self.open(request, Direction::Input, Cb::Input(Some(callback)))
    }

    fn open_output(
        &self,
        request: &StreamRequest,
        callback: Box<dyn OutputCallback>,
    ) -> Result<StreamHandle, BackendError> {
        self.open(request, Direction::Output, Cb::Output(Some(callback)))
    }
}

/// T-304: adds the [`Loopback`] into input slot `slot`'s block of frames `first..` (interleaved
/// `buf`, every channel). Runs outside the RT guard, like the device source.
fn mix_loopback(
    st: &State,
    lb: &Loopback,
    slot: usize,
    first: u64,
    channels: usize,
    buf: &mut [f32],
) {
    let input = &st.slots[slot];
    let Some(out) = st.slots.iter().rev().find(|s| {
        s.info.device == lb.output && s.info.direction == Direction::Output && s.record.is_some()
    }) else {
        return;
    };
    let Some(rec) = out.record.as_ref() else {
        return;
    };
    let gain = 10f64.powf(lb.gain_db / 20.0);
    let echo = lb.echo.map(|(d, g)| (i128::from(d), 10f64.powf(g / 20.0)));
    let noise = lb
        .noise
        .map(|(seed, db)| (seed, 10f64.powf(db / 20.0) * 3f64.sqrt()));
    let origin = i128::from(out.origin_ns);
    // The output sample truly heard at time `t` (nearest frame; 0 before the stream or past what
    // it has rendered).
    let heard = |t: i128| -> f64 {
        if t < origin {
            return 0.0;
        }
        let g = ((t - origin) as f64 * out.true_rate / 1e9).round() as usize;
        rec.samples.get(g).map_or(0.0, |&x| f64::from(x))
    };
    for (f, frame) in buf.chunks_exact_mut(channels.max(1)).enumerate() {
        let frame_idx = first + f as u64;
        let t = i128::from(input.frame_time(frame_idx));
        let muted = lb
            .mute
            .iter()
            .any(|&(a, b)| t >= i128::from(a) && t < i128::from(b));
        let tau = t - i128::from(lb.residual_ns);
        let mut v = if muted {
            0.0
        } else {
            gain * heard(tau) + echo.map_or(0.0, |(d, g)| gain * g * heard(tau - d))
        };
        if let Some((seed, amp)) = noise {
            let z = mix64(seed ^ frame_idx.wrapping_mul(0x9E37_79B9_7F4A_7C15));
            let unit = (z >> 11) as f64 * (1.0 / (1u64 << 53) as f64);
            v += (unit * 2.0 - 1.0) * amp;
        }
        for x in frame.iter_mut() {
            *x += v as f32;
        }
    }
}

/// SplitMix64 finalizer (stateless hash).
fn mix64(mut z: u64) -> u64 {
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// SplitMix64 PRNG.
#[derive(Clone, Debug)]
struct Rng(u64);

impl Rng {
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        mix64(self.0)
    }

    fn below(&mut self, n: u64) -> u64 {
        if n == 0 { 0 } else { self.next_u64() % n }
    }

    fn range(&mut self, lo: u32, hi: u32) -> u32 {
        if hi <= lo {
            return lo;
        }
        lo + self.below(u64::from(hi - lo) + 1) as u32
    }
}

//! Device selection, fallback rules, input-channel deinterleaving and hot-plug polling
//! (SPEC-001 §2.2, §2.5, §4; ADR-002 §1).
//!
//! - Pure functions: offered rates/buffer sizes, sample-rate fallback, buffer clamping, channel
//!   clamping, and [`resolve_devices`] (prefs + enumeration → applied config + notices).
//! - [`deinterleave_channel`] and [`MonoInput`]: the input callback keeps only the selected
//!   channel; changing it is an atomic store, no stream reopen.
//! - [`DeviceWatcher`] (steppable, for simulated-clock tests) and [`DevicePollThread`] (the
//!   ~1 s device-poll thread) turn enumerations into [`DeviceDiff`]s for the control thread.

use std::sync::Arc;
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::backend::{
    Backend, BackendError, BufferRequest, CapsState, DeviceInfo, DeviceKey, DeviceSnapshot,
    Direction, DirectionCaps, Enumerate, HostId, InputCallback, InputTimestamp, StreamRequest,
    frames_to_ns,
};
use crate::prefs::DevicePrefs;

/// Sample rates offered in Settings, intersected with the device's support (SPEC-001 §2.1).
pub const COMMON_SAMPLE_RATES_HZ: [u32; 6] = [44_100, 48_000, 88_200, 96_000, 176_400, 192_000];

/// Buffer sizes offered in Settings besides "Auto" (SPEC-001 §2.1).
pub const COMMON_BUFFER_SIZES: [u32; 6] = [64, 128, 256, 512, 1024, 2048];

/// Device-poll interval (SPEC-000 `DEVICE_POLL`).
pub const DEVICE_POLL_INTERVAL: Duration = Duration::from_secs(1);

/// Common rates the device supports; unsupported ones are hidden, not disabled.
pub fn offered_sample_rates(caps: &DirectionCaps) -> Vec<u32> {
    COMMON_SAMPLE_RATES_HZ
        .into_iter()
        .filter(|&r| caps.supports_rate(r))
        .collect()
}

/// Common buffer sizes within the device's range (all of them if the range is unknown).
/// "Auto" is always offered in addition.
pub fn offered_buffer_sizes(caps: &DirectionCaps) -> Vec<u32> {
    COMMON_BUFFER_SIZES
        .into_iter()
        .filter(|&n| {
            caps.buffer_range
                .is_none_or(|r| r.min_frames <= n && n <= r.max_frames)
        })
        .collect()
}

/// A requested value replaced by an applied one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fallback<T> {
    /// What the user asked for.
    pub requested: T,
    /// What is used instead.
    pub applied: T,
}

/// Sample-rate rule (SPEC-001 §2.2): the requested rate if supported, else the device default.
/// `None` requested = device default, no fallback.
pub fn resolve_sample_rate(
    requested_hz: Option<u32>,
    caps: &DirectionCaps,
) -> (u32, Option<Fallback<u32>>) {
    match requested_hz {
        Some(r) if caps.supports_rate(r) => (r, None),
        Some(r) => (
            caps.default_rate_hz,
            Some(Fallback {
                requested: r,
                applied: caps.default_rate_hz,
            }),
        ),
        None => (caps.default_rate_hz, None),
    }
}

/// Buffer-size rule (SPEC-001 §2.2): clamp to the nearest boundary of the device range.
/// `Auto` and unknown ranges pass through.
pub fn resolve_buffer_size(
    requested: BufferRequest,
    caps: &DirectionCaps,
) -> (BufferRequest, Option<Fallback<u32>>) {
    match (requested, caps.buffer_range) {
        (BufferRequest::Frames(n), Some(range)) => {
            let applied = n.clamp(range.min_frames, range.max_frames.max(range.min_frames));
            let fallback = (applied != n).then_some(Fallback {
                requested: n,
                applied,
            });
            (BufferRequest::Frames(applied), fallback)
        }
        (other, _) => (other, None),
    }
}

/// Input-channel rule: 1-based channel within `1..=max_channels`. A channel above the device's
/// count falls back to the last channel (with a notice); 0 is invalid and reads as 1.
pub fn resolve_input_channel(requested: u16, max_channels: u16) -> (u16, Option<Fallback<u16>>) {
    let max = max_channels.max(1);
    match requested {
        0 => (1, None),
        r if r > max => (
            max,
            Some(Fallback {
                requested: r,
                applied: max,
            }),
        ),
        r => (r, None),
    }
}

/// Device notices for the UI (SPEC-001 §2.2–§2.5). Text and i18n keys live in the UI layer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DeviceNotice {
    /// "48000 Hz isn't supported by *Device X* — using 44100 Hz."
    RateFallback {
        /// Device name.
        device: String,
        /// Requested rate.
        requested_hz: u32,
        /// Applied rate.
        applied_hz: u32,
    },
    /// Buffer size clamped to the device range.
    BufferFallback {
        /// Device name.
        device: String,
        /// Requested size.
        requested_frames: u32,
        /// Applied size.
        applied_frames: u32,
    },
    /// Saved input channel above the device's channel count.
    ChannelFallback {
        /// Device name.
        device: String,
        /// Requested 1-based channel.
        requested: u16,
        /// Applied 1-based channel.
        applied: u16,
    },
    /// "*Device X* not found — using default output *Device Y*." (`using` = `None`: no default.)
    DeviceNotFound {
        /// Direction.
        direction: Direction,
        /// Saved device name.
        saved: String,
        /// The host default used instead.
        using: Option<String>,
    },
    /// Saved host not available; using the default host.
    HostNotFound {
        /// Saved host.
        saved: HostId,
        /// Host used instead.
        using: HostId,
    },
    /// Device lost (SPEC-001 §2.4). The flags say what was interrupted.
    DeviceLost {
        /// Direction.
        direction: Direction,
        /// Device name.
        device: String,
        /// A recording was stopped (and its take kept).
        recording_stopped: bool,
        /// Playback was stopped.
        playback_stopped: bool,
        /// Output-only loss during recording: the recording goes on (SPEC-002 AC-16).
        recording_continues: bool,
    },
    /// "Output device reconnected." (auto-dismiss in the UI).
    DeviceReconnected {
        /// Direction.
        direction: Direction,
        /// Device name.
        device: String,
    },
    /// A non-fatal backend error on an open stream.
    BackendError {
        /// Direction.
        direction: Direction,
        /// Device name.
        device: String,
    },
    /// The document rate cannot be converted to the output stream's rate: playback is
    /// unavailable on it (never played at the wrong speed).
    ResampleUnavailable {
        /// Output device name.
        device: String,
        /// Document rate.
        doc_rate_hz: u32,
        /// Output stream rate.
        device_rate_hz: u32,
    },
}

/// Host rule (SPEC-001 §2.5): the saved host if available, else the default host with a notice.
pub fn resolve_host(
    saved: Option<HostId>,
    available: &[HostId],
    default: Option<HostId>,
) -> Option<(HostId, Option<DeviceNotice>)> {
    match saved {
        Some(h) if available.contains(&h) => Some((h, None)),
        Some(h) => default.map(|d| (d, Some(DeviceNotice::HostNotFound { saved: h, using: d }))),
        None => default.map(|d| (d, None)),
    }
}

/// Rate used to attempt opening a device whose capabilities could not be read when the prefs
/// name no rate. The open attempt decides; a failure is handled as device loss (SPEC-001 §2.2).
pub const UNKNOWN_CAPS_RATE_HZ: u32 = 48_000;

/// The applied input selection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppliedInput {
    /// Device.
    pub device: DeviceKey,
    /// 1-based channel to keep.
    pub channel: u16,
    /// The device's channel count (the input stream opens all of them); `None` while its
    /// capabilities are pending or unreadable.
    pub max_channels: Option<u16>,
}

/// Prefs applied to one host's current devices.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Resolution {
    /// Host.
    pub host: HostId,
    /// Input, `None` = no input device.
    pub input: Option<AppliedInput>,
    /// Output device, `None` = no output device available.
    pub output: Option<DeviceKey>,
    /// Applied device rate (from the output device, or the input device when there is no
    /// output). With unreadable capabilities: the preferred rate, else
    /// [`UNKNOWN_CAPS_RATE_HZ`]. `None` when there are no devices.
    pub sample_rate_hz: Option<u32>,
    /// Applied buffer request of the output device (or of the input device without output).
    pub buffer: BufferRequest,
    /// Applied buffer request of the input device, resolved against its own range.
    pub input_buffer: BufferRequest,
    /// A selected device's capabilities are still `Pending` (first enumeration phase): wait for
    /// the capability diff and resolve again before opening.
    pub caps_pending: bool,
    /// Notices produced by fallbacks, in the order they apply.
    pub notices: Vec<DeviceNotice>,
}

impl Resolution {
    /// Stream request for the output device.
    pub fn output_request(&self) -> Option<StreamRequest> {
        Some(StreamRequest {
            device: self.output.clone()?,
            sample_rate_hz: self.sample_rate_hz?,
            buffer: self.buffer,
            channels: None,
        })
    }

    /// Stream request for the input device at `sample_rate_hz` (the engine picks the input rate:
    /// document rate when supported, ADR-002 §5). All channels are opened.
    pub fn input_request(&self, sample_rate_hz: u32) -> Option<StreamRequest> {
        Some(StreamRequest {
            device: self.input.as_ref()?.device.clone(),
            sample_rate_hz,
            buffer: self.input_buffer,
            channels: None,
        })
    }
}

/// The device a saved name refers to (presence only, see [`DeviceSnapshot::lookup`]), else the
/// host default with a "not found" notice.
fn pick_device(
    saved: Option<&String>,
    dir: Direction,
    snap: &DeviceSnapshot,
    default_when_unset: bool,
    notices: &mut Vec<DeviceNotice>,
) -> Option<String> {
    match saved {
        Some(name) => match snap.lookup(name, dir) {
            Some(d) => Some(d.name.clone()),
            None => {
                let using = snap.default_for(dir).map(str::to_owned);
                notices.push(DeviceNotice::DeviceNotFound {
                    direction: dir,
                    saved: name.clone(),
                    using: using.clone(),
                });
                using
            }
        },
        None if default_when_unset => snap.default_for(dir).map(str::to_owned),
        None => None,
    }
}

/// Applies `prefs` to `snapshot` (the enumeration of `host`) per SPEC-001 §2.2/§2.5. Pure; never
/// modifies the prefs (saved intent is kept, AC-8).
///
/// A present device whose capabilities are unreadable is still used (no "not found" notice): its
/// rate is the preferred one (else [`UNKNOWN_CAPS_RATE_HZ`]) and the open attempt decides, a
/// failure counting as device loss (SPEC-001 §2.2). With `Pending` capabilities the result is
/// provisional ([`Resolution::caps_pending`]).
pub fn resolve_devices(prefs: &DevicePrefs, host: HostId, snap: &DeviceSnapshot) -> Resolution {
    let mut notices = Vec::new();
    let output = pick_device(
        prefs.output_device.as_ref(),
        Direction::Output,
        snap,
        true,
        &mut notices,
    );
    let input_name = pick_device(
        prefs.input_device.as_ref(),
        Direction::Input,
        snap,
        false,
        &mut notices,
    );
    let state = |name: &str, dir| snap.get(name).and_then(|d| d.caps_state(dir));
    let known = |name: &str, dir| state(name, dir).and_then(CapsState::known);
    let caps_pending = [
        (output.as_deref(), Direction::Output),
        (input_name.as_deref(), Direction::Input),
    ]
    .into_iter()
    .any(|(n, dir)| n.is_some_and(|n| matches!(state(n, dir), Some(CapsState::Pending))));

    let input = input_name.map(|name| {
        let (channel, max_channels) = match known(&name, Direction::Input) {
            Some(caps) => {
                let (channel, fb) = resolve_input_channel(prefs.input_channel, caps.max_channels);
                if let Some(fb) = fb {
                    notices.push(DeviceNotice::ChannelFallback {
                        device: name.clone(),
                        requested: fb.requested,
                        applied: fb.applied,
                    });
                }
                (channel, Some(caps.max_channels))
            }
            None => (prefs.input_channel.max(1), None),
        };
        AppliedInput {
            device: DeviceKey::new(host, name),
            channel,
            max_channels,
        }
    });

    // Rate/buffer basis: the output device, else the input device.
    let basis = output
        .as_ref()
        .map(|n| (n.clone(), known(n, Direction::Output)))
        .or_else(|| {
            let i = input.as_ref()?;
            Some((
                i.device.name.clone(),
                known(&i.device.name, Direction::Input),
            ))
        });

    let (sample_rate_hz, buffer) = match basis {
        Some((name, Some(caps))) => {
            let (rate, rate_fb) = resolve_sample_rate(prefs.sample_rate_hz, caps);
            if let Some(fb) = rate_fb {
                notices.push(DeviceNotice::RateFallback {
                    device: name.clone(),
                    requested_hz: fb.requested,
                    applied_hz: fb.applied,
                });
            }
            let (buffer, buf_fb) = resolve_buffer_size(prefs.buffer_size, caps);
            if let Some(fb) = buf_fb {
                notices.push(DeviceNotice::BufferFallback {
                    device: name,
                    requested_frames: fb.requested,
                    applied_frames: fb.applied,
                });
            }
            (Some(rate), buffer)
        }
        Some((_, None)) => (
            Some(prefs.sample_rate_hz.unwrap_or(UNKNOWN_CAPS_RATE_HZ)),
            prefs.buffer_size,
        ),
        None => (None, prefs.buffer_size),
    };

    // The input stream is separate: its buffer request is clamped to its own range.
    let input_buffer = match (&input, &output) {
        (Some(i), Some(_)) => match known(&i.device.name, Direction::Input) {
            Some(caps) => {
                let (b, fb) = resolve_buffer_size(prefs.buffer_size, caps);
                if let Some(fb) = fb {
                    notices.push(DeviceNotice::BufferFallback {
                        device: i.device.name.clone(),
                        requested_frames: fb.requested,
                        applied_frames: fb.applied,
                    });
                }
                b
            }
            None => prefs.buffer_size,
        },
        _ => buffer,
    };

    Resolution {
        host,
        input,
        output: output.map(|n| DeviceKey::new(host, n)),
        sample_rate_hz,
        buffer,
        input_buffer,
        caps_pending,
        notices,
    }
}

// ---------------------------------------------------------------------------------------------
// Input channel selection (RT)
// ---------------------------------------------------------------------------------------------

/// Copies channel `channel` (0-based) of the interleaved `data` into `out`. Returns the number of
/// frames written (limited by `out.len()`). Silence if `channel >= channels`. RT-safe.
#[inline]
pub fn deinterleave_channel(
    data: &[f32],
    channels: usize,
    channel: usize,
    out: &mut [f32],
) -> usize {
    if channels == 0 {
        return 0;
    }
    let frames = (data.len() / channels).min(out.len());
    if channel >= channels {
        out[..frames].fill(0.0);
        return frames;
    }
    for (o, frame) in out[..frames].iter_mut().zip(data.chunks_exact(channels)) {
        *o = frame[channel];
    }
    frames
}

/// Shared selected input channel (1-based), written by the control thread, read by the input
/// callback. Changing it takes effect at the next callback without reopening the stream.
#[derive(Clone, Debug)]
pub struct InputChannel(Arc<AtomicU16>);

impl InputChannel {
    /// A selector starting at `channel` (1-based; 0 reads as 1).
    pub fn new(channel: u16) -> Self {
        Self(Arc::new(AtomicU16::new(channel.max(1))))
    }

    /// Selects `channel` (1-based; 0 reads as 1).
    pub fn set(&self, channel: u16) {
        self.0.store(channel.max(1), Ordering::Relaxed);
    }

    /// The selected 1-based channel. RT-safe.
    #[inline]
    pub fn get(&self) -> u16 {
        self.0.load(Ordering::Relaxed)
    }
}

/// Input callback adapter: deinterleaves the selected channel (clamped to the device's last
/// channel) and hands mono blocks to `f`. Buffers larger than the preallocated scratch are split
/// into chunks with correspondingly advanced capture timestamps. RT-safe (no allocation after
/// construction).
pub struct MonoInput<F> {
    channel: InputChannel,
    sample_rate_hz: u32,
    scratch: Vec<f32>,
    f: F,
}

impl<F> MonoInput<F>
where
    F: FnMut(&[f32], InputTimestamp) + Send + 'static,
{
    /// `max_frames` sizes the scratch buffer (chunks above it are split, never reallocated).
    pub fn new(channel: InputChannel, sample_rate_hz: u32, max_frames: usize, f: F) -> Self {
        Self {
            channel,
            sample_rate_hz,
            scratch: vec![0.0; max_frames.max(1)],
            f,
        }
    }
}

impl<F> InputCallback for MonoInput<F>
where
    F: FnMut(&[f32], InputTimestamp) + Send + 'static,
{
    fn process(&mut self, data: &[f32], channels: usize, ts: InputTimestamp) {
        if channels == 0 {
            return;
        }
        let ch = usize::from(self.channel.get().max(1) - 1).min(channels - 1);
        let chunk_samples = self.scratch.len() * channels;
        let mut offset_frames: u64 = 0;
        for chunk in data.chunks(chunk_samples) {
            let n = deinterleave_channel(chunk, channels, ch, &mut self.scratch);
            let shift = frames_to_ns(offset_frames, self.sample_rate_hz);
            let chunk_ts = InputTimestamp {
                capture_ns: ts.capture_ns.saturating_add(shift),
                ..ts
            };
            (self.f)(&self.scratch[..n], chunk_ts);
            offset_frames += n as u64;
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Snapshots, diffs, polling
// ---------------------------------------------------------------------------------------------

/// Changes between two enumerations of one host (posted to the control thread).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceDiff {
    /// Host.
    pub host: HostId,
    /// `true`: first pass for this host — replace the whole list (`added` holds every device).
    pub full: bool,
    /// Devices that appeared.
    pub added: Vec<DeviceInfo>,
    /// Names of devices that disappeared.
    pub removed: Vec<String>,
    /// Devices whose capabilities changed.
    pub changed: Vec<DeviceInfo>,
    /// Current host default input.
    pub default_input: Option<String>,
    /// Current host default output.
    pub default_output: Option<String>,
}

impl DeviceDiff {
    /// True if nothing changed.
    pub fn is_empty(&self) -> bool {
        !self.full && self.added.is_empty() && self.removed.is_empty() && self.changed.is_empty()
    }
}

/// Diff from `old` (None = nothing known yet) to `new`. Returns `None` if nothing changed.
pub fn diff_snapshots(
    host: HostId,
    old: Option<&DeviceSnapshot>,
    new: &DeviceSnapshot,
) -> Option<DeviceDiff> {
    let Some(old) = old else {
        return Some(DeviceDiff {
            host,
            full: true,
            added: new.devices.clone(),
            removed: Vec::new(),
            changed: Vec::new(),
            default_input: new.default_input.clone(),
            default_output: new.default_output.clone(),
        });
    };
    let mut diff = DeviceDiff {
        host,
        full: false,
        added: Vec::new(),
        removed: Vec::new(),
        changed: Vec::new(),
        default_input: new.default_input.clone(),
        default_output: new.default_output.clone(),
    };
    for d in &new.devices {
        match old.get(&d.name) {
            None => diff.added.push(d.clone()),
            Some(prev) if prev != d => diff.changed.push(d.clone()),
            Some(_) => {}
        }
    }
    diff.removed = old
        .devices
        .iter()
        .filter(|d| new.get(&d.name).is_none())
        .map(|d| d.name.clone())
        .collect();
    let defaults_changed =
        old.default_input != new.default_input || old.default_output != new.default_output;
    (!diff.is_empty() || defaults_changed).then_some(diff)
}

/// The control thread's device list for the selected host, maintained from diffs.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DeviceList {
    /// Host the list belongs to (`None` until the first diff).
    pub host: Option<HostId>,
    /// Current snapshot.
    pub snapshot: DeviceSnapshot,
}

impl DeviceList {
    /// Applies a diff. A diff for a different host (or a `full` one) replaces the list.
    pub fn apply(&mut self, diff: &DeviceDiff) {
        if diff.full || self.host != Some(diff.host) {
            self.host = Some(diff.host);
            self.snapshot.devices.clear();
        }
        let devices = &mut self.snapshot.devices;
        devices.retain(|d| !diff.removed.contains(&d.name));
        for c in &diff.changed {
            if let Some(d) = devices.iter_mut().find(|d| d.name == c.name) {
                *d = c.clone();
            }
        }
        for a in &diff.added {
            if !devices.iter().any(|d| d.name == a.name) {
                devices.push(a.clone());
            }
        }
        self.snapshot.default_input = diff.default_input.clone();
        self.snapshot.default_output = diff.default_output.clone();
    }

    /// True if `name` refers to a present device supporting `dir`, whatever its capability state
    /// (see [`DeviceSnapshot::lookup`]: a base name still matches after an identical second device
    /// appeared, so that never looks like a loss).
    pub fn is_present(&self, name: &str, dir: Direction) -> bool {
        self.snapshot.is_present(name, dir)
    }
}

/// One host's enumeration state; `poll` produces diffs. Steppable (tests drive it on a simulated
/// clock); [`DevicePollThread`] drives it every ~1 s.
#[derive(Debug)]
pub struct DeviceWatcher {
    host: HostId,
    current: Option<DeviceSnapshot>,
}

impl DeviceWatcher {
    /// Watches `host`; the first poll reports everything as a `full` diff.
    pub fn new(host: HostId) -> Self {
        Self {
            host,
            current: None,
        }
    }

    /// The watched host.
    pub fn host(&self) -> HostId {
        self.host
    }

    /// Switches host; the next poll reports a `full` diff.
    pub fn set_host(&mut self, host: HostId) {
        self.host = host;
        self.current = None;
    }

    /// The last successful enumeration.
    pub fn snapshot(&self) -> Option<&DeviceSnapshot> {
        self.current.as_ref()
    }

    /// Enumerates once. `Ok(None)`: no change. On error the previous snapshot is kept (a transient
    /// enumeration failure must not look like every device vanishing; stream flags report real
    /// loss immediately).
    pub fn poll(
        &mut self,
        backend: &dyn Backend,
        mode: Enumerate,
    ) -> Result<Option<DeviceDiff>, BackendError> {
        let mut snap = backend.enumerate(self.host, mode)?;
        snap.normalize();
        let diff = diff_snapshots(self.host, self.current.as_ref(), &snap);
        self.current = Some(snap);
        Ok(diff)
    }

    /// One poll pass, two-phase: a [`Enumerate::Quick`] listing (fast — names first, new
    /// devices' capabilities possibly `Pending`), then, if anything is pending or `mode` is
    /// `Fresh`, a capability read with `mode` (a `changed` diff). Returns the diffs in order and
    /// the error of the phase that failed, if any (diffs already produced are still returned).
    pub fn pass(
        &mut self,
        backend: &dyn Backend,
        mode: Enumerate,
    ) -> (Vec<DeviceDiff>, Option<BackendError>) {
        let mut diffs = Vec::new();
        match self.poll(backend, Enumerate::Quick) {
            Ok(d) => diffs.extend(d),
            Err(e) => return (diffs, Some(e)),
        }
        let pending = self
            .current
            .as_ref()
            .is_some_and(DeviceSnapshot::has_pending);
        if pending || mode == Enumerate::Fresh {
            let read = if mode == Enumerate::Quick {
                Enumerate::Cached
            } else {
                mode
            };
            match self.poll(backend, read) {
                Ok(d) => diffs.extend(d),
                Err(e) => return (diffs, Some(e)),
            }
        }
        (diffs, None)
    }
}

/// Messages from the device-poll thread.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PollEvent {
    /// Available hosts (at start and after every rescan).
    Hosts {
        /// Hosts compiled in and reachable.
        available: Vec<HostId>,
        /// Default host among them.
        default: Option<HostId>,
    },
    /// Device list changes.
    Devices(DeviceDiff),
    /// Enumeration failed; the previous list stays valid.
    Error {
        /// Host.
        host: HostId,
        /// Error.
        error: BackendError,
    },
}

enum PollCmd {
    Rescan,
    SetHost(HostId),
    Stop,
}

/// The device-poll thread (ADR-002 §1): enumerates the selected host every `interval` and posts
/// diffs through `sink`; [`rescan`](Self::rescan) forces an immediate full pass. Stops on drop.
pub struct DevicePollThread {
    tx: mpsc::Sender<PollCmd>,
    join: Option<JoinHandle<()>>,
}

impl DevicePollThread {
    /// Spawns the thread. The first pass (hosts + devices) runs immediately.
    pub fn spawn<S>(
        backend: Arc<dyn Backend>,
        host: HostId,
        interval: Duration,
        mut sink: S,
    ) -> std::io::Result<Self>
    where
        S: FnMut(PollEvent) + Send + 'static,
    {
        let (tx, rx) = mpsc::channel();
        let join = std::thread::Builder::new()
            .name("vox-device-poll".into())
            .spawn(move || {
                let mut watcher = DeviceWatcher::new(host);
                let pass = |w: &mut DeviceWatcher, mode, hosts: bool, sink: &mut S| {
                    if hosts {
                        let available = backend.hosts();
                        let default = crate::backend::choose_default_host(&available);
                        sink(PollEvent::Hosts { available, default });
                    }
                    let (diffs, error) = w.pass(backend.as_ref(), mode);
                    for diff in diffs {
                        sink(PollEvent::Devices(diff));
                    }
                    if let Some(error) = error {
                        sink(PollEvent::Error {
                            host: w.host(),
                            error,
                        });
                    }
                };
                pass(&mut watcher, Enumerate::Cached, true, &mut sink);
                let mut next = Instant::now() + interval;
                loop {
                    let wait = next.saturating_duration_since(Instant::now());
                    match rx.recv_timeout(wait) {
                        Err(RecvTimeoutError::Timeout) => {
                            pass(&mut watcher, Enumerate::Cached, false, &mut sink);
                            next += interval;
                            let now = Instant::now();
                            if next < now {
                                next = now + interval;
                            }
                        }
                        Ok(PollCmd::Rescan) => {
                            pass(&mut watcher, Enumerate::Fresh, true, &mut sink);
                            next = Instant::now() + interval;
                        }
                        Ok(PollCmd::SetHost(h)) => {
                            watcher.set_host(h);
                            pass(&mut watcher, Enumerate::Cached, false, &mut sink);
                            next = Instant::now() + interval;
                        }
                        Ok(PollCmd::Stop) | Err(RecvTimeoutError::Disconnected) => break,
                    }
                }
            })?;
        Ok(Self {
            tx,
            join: Some(join),
        })
    }

    /// Manual rescan (Settings "Rescan" button): hosts + a fresh enumeration, off the UI thread.
    pub fn rescan(&self) {
        let _ = self.tx.send(PollCmd::Rescan);
    }

    /// Switches the polled host (the next event is a `full` diff).
    pub fn set_host(&self, host: HostId) {
        let _ = self.tx.send(PollCmd::SetHost(host));
    }
}

impl Drop for DevicePollThread {
    fn drop(&mut self) {
        let _ = self.tx.send(PollCmd::Stop);
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::{BufferRange, RateRange};

    fn caps(rates: &[u32], default: u32, buf: Option<(u32, u32)>) -> DirectionCaps {
        DirectionCaps {
            max_channels: 2,
            default_channels: 2,
            rates: rates.iter().map(|&r| RateRange::exactly(r)).collect(),
            default_rate_hz: default,
            buffer_range: buf.map(|(min_frames, max_frames)| BufferRange {
                min_frames,
                max_frames,
            }),
        }
    }

    #[test]
    fn offered_values_are_intersections() {
        let c = caps(&[44_100, 48_000, 32_000], 48_000, Some((128, 1024)));
        assert_eq!(offered_sample_rates(&c), vec![44_100, 48_000]);
        assert_eq!(offered_buffer_sizes(&c), vec![128, 256, 512, 1024]);
        let c = caps(&[48_000], 48_000, None);
        assert_eq!(offered_buffer_sizes(&c), COMMON_BUFFER_SIZES.to_vec());
        let range = DirectionCaps {
            rates: vec![RateRange {
                min_hz: 8_000,
                max_hz: 96_000,
            }],
            ..c
        };
        assert_eq!(
            offered_sample_rates(&range),
            vec![44_100, 48_000, 88_200, 96_000]
        );
    }

    /// SPEC-001 AC-3 (unit): unsupported 192 kHz falls back to the device default.
    #[test]
    fn sample_rate_fallback() {
        let c = caps(&[44_100, 48_000], 44_100, None);
        assert_eq!(resolve_sample_rate(Some(48_000), &c), (48_000, None));
        assert_eq!(
            resolve_sample_rate(Some(192_000), &c),
            (
                44_100,
                Some(Fallback {
                    requested: 192_000,
                    applied: 44_100
                })
            )
        );
        assert_eq!(resolve_sample_rate(None, &c), (44_100, None));
    }

    /// SPEC-001 AC-4 (unit): clamp to the nearest range boundary.
    #[test]
    fn buffer_size_clamping() {
        let c = caps(&[48_000], 48_000, Some((128, 1024)));
        let r = |n| resolve_buffer_size(BufferRequest::Frames(n), &c);
        assert_eq!(
            r(2048),
            (
                BufferRequest::Frames(1024),
                Some(Fallback {
                    requested: 2048,
                    applied: 1024
                })
            )
        );
        assert_eq!(
            r(64),
            (
                BufferRequest::Frames(128),
                Some(Fallback {
                    requested: 64,
                    applied: 128
                })
            )
        );
        assert_eq!(r(128), (BufferRequest::Frames(128), None));
        assert_eq!(r(1024), (BufferRequest::Frames(1024), None));
        assert_eq!(
            resolve_buffer_size(BufferRequest::Auto, &c),
            (BufferRequest::Auto, None)
        );
        let unknown = caps(&[48_000], 48_000, None);
        assert_eq!(
            resolve_buffer_size(BufferRequest::Frames(4096), &unknown),
            (BufferRequest::Frames(4096), None)
        );
    }

    #[test]
    fn input_channel_clamping() {
        assert_eq!(resolve_input_channel(2, 2), (2, None));
        assert_eq!(
            resolve_input_channel(2, 1),
            (
                1,
                Some(Fallback {
                    requested: 2,
                    applied: 1
                })
            )
        );
        assert_eq!(resolve_input_channel(0, 4), (1, None));
    }

    #[test]
    fn host_resolution() {
        use HostId::*;
        assert_eq!(
            resolve_host(Some(Jack), &[Alsa, Jack], Some(Alsa)),
            Some((Jack, None))
        );
        assert_eq!(
            resolve_host(Some(PipeWire), &[Alsa], Some(Alsa)),
            Some((
                Alsa,
                Some(DeviceNotice::HostNotFound {
                    saved: PipeWire,
                    using: Alsa
                })
            ))
        );
        assert_eq!(resolve_host(None, &[Alsa], Some(Alsa)), Some((Alsa, None)));
        assert_eq!(resolve_host(None, &[], None), None);
    }

    /// SPEC-001 AC-2 (unit): deinterleave keeps only the selected channel.
    #[test]
    fn deinterleave_selects_one_channel() {
        let data = [1.0, -1.0, 2.0, -2.0, 3.0, -3.0];
        let mut out = [9.0f32; 3];
        assert_eq!(deinterleave_channel(&data, 2, 0, &mut out), 3);
        assert_eq!(out.map(f32::to_bits), [1.0f32, 2.0, 3.0].map(f32::to_bits));
        assert_eq!(deinterleave_channel(&data, 2, 1, &mut out), 3);
        assert_eq!(
            out.map(f32::to_bits),
            [-1.0f32, -2.0, -3.0].map(f32::to_bits)
        );
        assert_eq!(deinterleave_channel(&data, 2, 5, &mut out), 3);
        assert!(out.iter().all(|&x| x.to_bits() == 0));
        let mut short = [0.0f32; 2];
        assert_eq!(deinterleave_channel(&data, 2, 0, &mut short), 2);
        assert_eq!(deinterleave_channel(&data, 0, 0, &mut short), 0);
    }

    #[test]
    fn mono_input_chunks_large_buffers_with_advancing_timestamps() {
        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let seen2 = seen.clone();
        let sel = InputChannel::new(2);
        let mut mono = MonoInput::new(sel.clone(), 48_000, 4, move |x: &[f32], ts| {
            seen2.lock().unwrap().push((x.to_vec(), ts.capture_ns));
        });
        // 6 frames, 2 channels: ch2 = frame index.
        let data: Vec<f32> = (0..6).flat_map(|i| [100.0, i as f32]).collect();
        let ts = InputTimestamp {
            now_ns: 5,
            callback_ns: 10,
            capture_ns: 1_000,
        };
        InputCallback::process(&mut mono, &data, 2, ts);
        sel.set(1);
        InputCallback::process(&mut mono, &data[..4], 2, ts);
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 3);
        assert_eq!(seen[0].0, vec![0.0, 1.0, 2.0, 3.0]);
        assert_eq!(seen[0].1, 1_000);
        assert_eq!(seen[1].0, vec![4.0, 5.0]);
        assert_eq!(seen[1].1, 1_000 + frames_to_ns(4, 48_000));
        assert_eq!(seen[2].0, vec![100.0, 100.0]);
    }

    fn snap(devices: Vec<DeviceInfo>) -> DeviceSnapshot {
        let mut s = DeviceSnapshot {
            default_input: devices
                .iter()
                .find(|d| d.supports(Direction::Input))
                .map(|d| d.name.clone()),
            default_output: devices
                .iter()
                .find(|d| d.supports(Direction::Output))
                .map(|d| d.name.clone()),
            devices,
        };
        s.normalize();
        s
    }

    fn known(rates: &[u32], default: u32, buf: Option<(u32, u32)>) -> CapsState {
        CapsState::Known(caps(rates, default, buf))
    }

    /// SPEC-001 §2.2: a present device with unreadable capabilities is not "not found".
    #[test]
    fn unreadable_or_pending_capabilities_keep_the_device() {
        let s = snap(vec![
            DeviceInfo::new("Default").with_output(known(&[48_000], 48_000, None)),
            DeviceInfo::new("Busy DAC").with_output(CapsState::Unavailable),
            DeviceInfo::new("New Mic").with_input(CapsState::Pending),
        ]);
        let prefs = DevicePrefs {
            output_device: Some("Busy DAC".into()),
            input_device: Some("New Mic".into()),
            input_channel: 2,
            ..DevicePrefs::default()
        };
        let res = resolve_devices(&prefs, HostId::Alsa, &s);
        assert!(res.notices.is_empty(), "{:?}", res.notices);
        assert_eq!(res.output, Some(DeviceKey::new(HostId::Alsa, "Busy DAC")));
        assert_eq!(res.sample_rate_hz, Some(UNKNOWN_CAPS_RATE_HZ));
        let input = res.input.as_ref().unwrap();
        assert_eq!((input.channel, input.max_channels), (2, None));
        assert!(res.caps_pending);
        let prefs = DevicePrefs {
            sample_rate_hz: Some(44_100),
            ..prefs
        };
        assert_eq!(
            resolve_devices(&prefs, HostId::Alsa, &s).sample_rate_hz,
            Some(44_100)
        );
    }

    /// The input stream's buffer size is clamped to the input device's range, with a notice.
    #[test]
    fn input_buffer_is_resolved_against_the_input_device() {
        let s = snap(vec![
            DeviceInfo::new("DAC").with_output(known(&[48_000], 48_000, Some((128, 1024)))),
            DeviceInfo::new("Mic").with_input(known(&[48_000], 48_000, Some((256, 4096)))),
        ]);
        let prefs = DevicePrefs {
            input_device: Some("Mic".into()),
            buffer_size: BufferRequest::Frames(128),
            ..DevicePrefs::default()
        };
        let res = resolve_devices(&prefs, HostId::Alsa, &s);
        assert_eq!(res.buffer, BufferRequest::Frames(128));
        assert_eq!(res.input_buffer, BufferRequest::Frames(256));
        assert_eq!(
            res.notices,
            vec![DeviceNotice::BufferFallback {
                device: "Mic".into(),
                requested_frames: 128,
                applied_frames: 256
            }]
        );
        assert_eq!(
            res.input_request(48_000).unwrap().buffer,
            BufferRequest::Frames(256)
        );
        // Input only: one resolution, one notice.
        let s = snap(vec![DeviceInfo::new("Mic").with_input(known(
            &[48_000],
            48_000,
            Some((256, 4096)),
        ))]);
        let res = resolve_devices(&prefs, HostId::Alsa, &s);
        assert_eq!(res.input_buffer, BufferRequest::Frames(256));
        assert_eq!(res.notices.len(), 1);
    }

    /// A saved base name resolves to its (now suffixed) device without a notice.
    #[test]
    fn saved_base_name_survives_a_name_collision() {
        let twin = |id: &str| DeviceInfo {
            base_name: "USB Mic".into(),
            id: id.into(),
            ..DeviceInfo::new(format!("USB Mic ({id})")).with_input(known(&[48_000], 48_000, None))
        };
        let s = snap(vec![twin("a"), twin("b")]);
        let prefs = DevicePrefs {
            input_device: Some("USB Mic".into()),
            ..DevicePrefs::default()
        };
        let res = resolve_devices(&prefs, HostId::PipeWire, &s);
        assert!(res.notices.is_empty());
        assert_eq!(res.input.unwrap().device.name, "USB Mic (a)");
    }

    #[test]
    fn diff_and_apply_reproduce_the_new_snapshot() {
        let dev = |n: &str, r: u32| DeviceInfo::new(n).with_output(known(&[r], r, None));
        let a = DeviceSnapshot {
            devices: vec![dev("A", 48_000), dev("B", 48_000)],
            default_input: None,
            default_output: Some("A".into()),
        };
        let b = DeviceSnapshot {
            devices: vec![dev("B", 44_100), dev("C", 48_000)],
            default_input: None,
            default_output: Some("C".into()),
        };
        let mut list = DeviceList::default();
        let first = diff_snapshots(HostId::Alsa, None, &a).unwrap();
        assert!(first.full);
        list.apply(&first);
        assert_eq!(list.snapshot, a);
        assert!(diff_snapshots(HostId::Alsa, Some(&a), &a).is_none());
        let d = diff_snapshots(HostId::Alsa, Some(&a), &b).unwrap();
        assert_eq!(d.removed, vec!["A".to_string()]);
        assert_eq!(d.added.len(), 1);
        assert_eq!(d.changed.len(), 1);
        list.apply(&d);
        assert_eq!(list.snapshot, b);
        assert!(list.is_present("C", Direction::Output));
        assert!(!list.is_present("C", Direction::Input));
    }
}

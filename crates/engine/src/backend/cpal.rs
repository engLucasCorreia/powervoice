//! cpal 0.18 backend (feature `backend-cpal`; ADR-001 §3: cpal is confined to this crate).
//!
//! Host handling (SPEC-001 §2.1):
//! - Linux: PipeWire (cpal feature `pipewire`) is the default when its server is reachable, else
//!   ALSA; JACK is listed when a JACK server (or PipeWire's JACK shim) accepts a client.
//! - cpal's PipeWire host snapshots its device list when it is created, so every enumeration of
//!   PipeWire builds a fresh host. ALSA/WASAPI/CoreAudio hosts re-enumerate on each call and are
//!   cached. JACK's two synthetic devices are created with the host (they open JACK clients), so
//!   its host is cached and only rebuilt on a fresh (manual) enumeration.
//!
//! Device names (unique `(host, name)` keys, stable across passes):
//! - ALSA: always `"<description> (<PCM id>)"`, because ALSA exposes many views of one card
//!   (`hw:`, `plughw:`, `front:`, …) under the same description.
//! - Other hosts: the description; devices sharing it each get ` (<id>)` (PipeWire: the node
//!   name). A name saved before such a collision still resolves ([`DeviceSnapshot::lookup`]).
//!
//! Enumeration is two-phase ([`Enumerate::Quick`], then `Cached`/`Fresh`): reading ALSA
//! capabilities opens every PCM (~2–3 s for 60 entries), so the quick phase lists names with
//! capabilities from earlier passes or `Pending`. PipeWire/JACK capabilities cost nothing and are
//! read in the quick phase. A failed read is `Unavailable` (the device stays present); a fresh
//! read that fails keeps previously known capabilities (we may be streaming on the device).
//!
//! ALSA plug-layer devices (`plughw`, `sysdefault`, …) accept any rate and 1…64 channels, and
//! cpal's JACK devices offer 1…64 ports; their channel count comes from the default configuration
//! instead.
//!
//! Callbacks always see `f32`. Other device formats are converted through a scratch buffer sized
//! from the opened buffer size (callback contract: see [`super`]). The error callback only updates
//! the stream's atomic [`StreamStatus`].

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, MutexGuard};

use ::cpal as cp;
use cp::traits::{DeviceTrait, HostTrait, StreamTrait};

use super::{
    Backend, BackendError, BufferRange, BufferRequest, CapsState, DeviceInfo, DeviceKey,
    DeviceSnapshot, Direction, DirectionCaps, Enumerate, ErrorClass, HostId, InputCallback,
    InputTimestamp, OutputCallback, OutputTimestamp, RateRange, StreamHandle, StreamId, StreamInfo,
    StreamRequest, StreamStatus, app_epoch, app_now_ns, frames_to_ns,
};

/// Conversion scratch (frames) for non-f32 formats with `Auto` buffers: the device's maximum
/// buffer size, clamped to this range.
const SCRATCH_MIN_FRAMES: usize = 1024;
const SCRATCH_MAX_FRAMES: usize = 16_384;
/// Upper bound for fixed buffer requests.
const SCRATCH_FIXED_MAX_FRAMES: usize = 65_536;

/// Capability states of one device (never `Pending`).
#[derive(Clone, Debug, Default)]
struct CachedCaps {
    input: Option<CapsState>,
    output: Option<CapsState>,
}

#[derive(Default)]
struct Inner {
    hosts: HashMap<HostId, cp::Host>,
    /// Per (host, device id).
    caps: HashMap<(HostId, String), CachedCaps>,
    /// cpal ids per (host, device id).
    ids: HashMap<(HostId, String), cp::DeviceId>,
    /// Last enumeration per host, to resolve names at open.
    snapshots: HashMap<HostId, DeviceSnapshot>,
    next_stream: u64,
}

/// Real audio devices through cpal.
pub struct CpalBackend {
    inner: Mutex<Inner>,
}

impl Default for CpalBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for CpalBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CpalBackend").finish_non_exhaustive()
    }
}

/// Our host id for a cpal host id (`None` for hosts outside SPEC-001, e.g. ASIO).
fn ours(id: cp::HostId) -> Option<HostId> {
    id.to_string().parse().ok()
}

/// cpal's host id for ours, if compiled in.
fn theirs(id: HostId) -> Option<cp::HostId> {
    cp::ALL_HOSTS.iter().copied().find(|c| ours(*c) == Some(id))
}

fn map_err(e: &cp::Error, host: HostId, key: Option<&DeviceKey>) -> BackendError {
    use cp::ErrorKind as K;
    match (e.kind(), key) {
        (K::HostUnavailable, _) => BackendError::HostUnavailable(host),
        (K::DeviceNotAvailable, Some(k)) => BackendError::DeviceNotFound(k.clone()),
        (K::DeviceBusy, Some(k)) => BackendError::DeviceBusy(k.clone()),
        (K::UnsupportedConfig | K::InvalidInput, _) => {
            BackendError::UnsupportedConfig(e.to_string())
        }
        _ => BackendError::Backend(e.to_string()),
    }
}

/// Error-callback classification (ADR-002 §7).
pub(crate) fn classify(kind: cp::ErrorKind) -> ErrorClass {
    use cp::ErrorKind as K;
    match kind {
        K::DeviceNotAvailable | K::HostUnavailable | K::StreamInvalidated => ErrorClass::DeviceLost,
        K::Xrun => ErrorClass::Xrun,
        K::DeviceChanged | K::RealtimeDenied => ErrorClass::Benign,
        _ => ErrorClass::Other,
    }
}

/// Preference among device sample formats we can convert (higher is better).
pub(crate) fn format_rank(f: cp::SampleFormat) -> Option<u8> {
    use cp::SampleFormat as F;
    Some(match f {
        F::F32 => 8,
        F::I32 => 7,
        F::I24 => 6,
        F::F64 => 5,
        F::I16 => 4,
        F::U16 => 3,
        F::I8 => 2,
        F::U8 => 1,
        _ => return None,
    })
}

/// Unique key names for `(base name, id)` entries of one host (see the module docs).
pub(crate) fn unique_names(host: HostId, entries: &[(String, String)]) -> Vec<String> {
    let mut count: HashMap<&str, usize> = HashMap::new();
    for (name, _) in entries {
        *count.entry(name.as_str()).or_default() += 1;
    }
    entries
        .iter()
        .map(|(name, id)| {
            if host == HostId::Alsa || count[name.as_str()] > 1 {
                format!("{name} ({id})")
            } else {
                name.clone()
            }
        })
        .collect()
}

/// "Follow the system default" pseudo-devices.
pub(crate) fn is_system_default(host: HostId, id: &str) -> bool {
    match host {
        HostId::PipeWire => matches!(id, "input_default" | "output_default" | "sink_default"),
        HostId::Alsa => id == "default",
        _ => false,
    }
}

/// PipeWire exposes every `Audio/Sink` node as duplex: its capture side is the sink's monitor.
/// Real capture nodes are `Input`, true duplex nodes are named `*input*`/`*duplex*`.
pub(crate) fn is_monitor_input(host: HostId, id: &str, dir: cp::DeviceDirection) -> bool {
    host == HostId::PipeWire
        && (id == "sink_default"
            || (dir == cp::DeviceDirection::Duplex
                && !id.contains("input")
                && !id.contains("duplex")))
}

/// A synthetic channel range: an ALSA plug-layer device (accepts any rate, reported up to
/// `u32::MAX`, and 1…64 channels) or JACK (cpal offers 1…64 ports on its synthetic devices).
/// Such devices are opened with their default channel count.
pub(crate) fn is_plug_layer(host: HostId, configs: &[cp::SupportedStreamConfigRange]) -> bool {
    match host {
        HostId::Alsa => configs.iter().any(|c| c.max_sample_rate() == u32::MAX),
        HostId::Jack => true,
        _ => false,
    }
}

/// Conversion scratch size for a stream opened with `size` on a config with `range`.
pub(crate) fn scratch_frames(size: cp::BufferSize, range: cp::SupportedBufferSize) -> usize {
    match (size, range) {
        (cp::BufferSize::Fixed(n), _) => {
            (n as usize).clamp(SCRATCH_MIN_FRAMES, SCRATCH_FIXED_MAX_FRAMES)
        }
        (cp::BufferSize::Default, cp::SupportedBufferSize::Range { max, .. }) => {
            (max as usize).clamp(SCRATCH_MIN_FRAMES, SCRATCH_MAX_FRAMES)
        }
        (cp::BufferSize::Default, cp::SupportedBufferSize::Unknown) => SCRATCH_MAX_FRAMES,
    }
}

/// Capabilities from cpal's supported configurations (formats we can't convert are ignored).
/// `plug_layer`: the channel range is a plug-layer artefact; use the default channel count.
pub(crate) fn caps_from(
    configs: &[cp::SupportedStreamConfigRange],
    default: Option<cp::SupportedStreamConfig>,
    plug_layer: bool,
) -> Option<DirectionCaps> {
    let usable: Vec<&cp::SupportedStreamConfigRange> = configs
        .iter()
        .filter(|c| c.channels() > 0 && format_rank(c.sample_format()).is_some())
        .collect();
    let best = usable
        .iter()
        .copied()
        .max_by(|a, b| a.cmp_default_heuristics(b))?;
    let mut rates: Vec<RateRange> = usable
        .iter()
        .map(|c| RateRange {
            min_hz: c.min_sample_rate(),
            max_hz: c.max_sample_rate(),
        })
        .collect();
    DirectionCaps::normalize_rates(&mut rates);
    let (default_channels, default_rate_hz, default_buf) = match default {
        Some(d) => (d.channels(), d.sample_rate(), *d.buffer_size()),
        None => {
            let rate = best
                .try_with_standard_sample_rate()
                .map_or(best.max_sample_rate(), |c| c.sample_rate());
            (best.channels(), rate, *best.buffer_size())
        }
    };
    let to_range = |b: cp::SupportedBufferSize| match b {
        cp::SupportedBufferSize::Range { min, max } if min <= max && max > 0 => Some(BufferRange {
            min_frames: min.max(1),
            max_frames: max,
        }),
        _ => None,
    };
    let buffer_range = to_range(default_buf).or_else(|| {
        usable
            .iter()
            .filter_map(|c| to_range(*c.buffer_size()))
            .reduce(|a, b| BufferRange {
                min_frames: a.min_frames.min(b.min_frames),
                max_frames: a.max_frames.max(b.max_frames),
            })
    });
    let max_channels = if plug_layer {
        default_channels.max(1)
    } else {
        usable.iter().map(|c| c.channels()).max().unwrap_or(1)
    };
    Some(DirectionCaps {
        max_channels,
        default_channels: default_channels.min(max_channels),
        rates,
        default_rate_hz,
        buffer_range,
    })
}

/// A stream configuration chosen by [`pick_config`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Picked {
    pub(crate) channels: u16,
    pub(crate) format: cp::SampleFormat,
    pub(crate) size: cp::BufferSize,
    pub(crate) applied: BufferRequest,
    pub(crate) range: cp::SupportedBufferSize,
}

/// Picks a stream configuration: exact channel match first, then the best sample format; a fixed
/// buffer size is clamped to the config's range.
pub(crate) fn pick_config(
    configs: &[cp::SupportedStreamConfigRange],
    rate_hz: u32,
    channels: u16,
    buffer: BufferRequest,
) -> Option<Picked> {
    let c = configs
        .iter()
        .filter(|c| c.contains_rate(rate_hz) && c.channels() > 0)
        .filter_map(|c| format_rank(c.sample_format()).map(|r| (c, r)))
        .max_by_key(|(c, r)| (c.channels() == channels, *r, c.channels()))?
        .0;
    let range = *c.buffer_size();
    let (size, applied) = match (buffer, range) {
        (BufferRequest::Auto, _) => (cp::BufferSize::Default, BufferRequest::Auto),
        (BufferRequest::Frames(n), cp::SupportedBufferSize::Range { min, max }) if min <= max => {
            let n = n.clamp(min.max(1), max);
            (cp::BufferSize::Fixed(n), BufferRequest::Frames(n))
        }
        (BufferRequest::Frames(n), _) => (cp::BufferSize::Fixed(n), BufferRequest::Frames(n)),
    };
    Some(Picked {
        channels: c.channels(),
        format: c.sample_format(),
        size,
        applied,
        range,
    })
}

#[inline]
fn ns(i: cp::StreamInstant) -> u64 {
    i.as_nanos() as u64
}

/// RT side of an input stream: optional format conversion, then the user callback.
pub(crate) struct InputWrap {
    cb: Box<dyn InputCallback>,
    scratch: Vec<f32>,
    channels: usize,
    rate_hz: u32,
    status: Arc<StreamStatus>,
}

impl InputWrap {
    /// `scratch_frames`: conversion buffer (frames); callbacks up to it make exactly one call.
    pub(crate) fn new(
        cb: Box<dyn InputCallback>,
        channels: usize,
        rate_hz: u32,
        scratch_frames: usize,
        status: Arc<StreamStatus>,
    ) -> Self {
        Self {
            cb,
            scratch: vec![0.0; scratch_frames.max(1) * channels.max(1)],
            channels: channels.max(1),
            rate_hz,
            status,
        }
    }

    #[inline]
    pub(crate) fn run_f32(&mut self, data: &[f32], ts: InputTimestamp) {
        self.status.count_callback();
        self.cb.process(data, self.channels, ts);
    }

    /// Converts `data` chunk by chunk (RT-safe).
    pub(crate) fn run_converted<T: Copy>(&mut self, data: &[T], ts: InputTimestamp)
    where
        f32: cp::FromSample<T>,
    {
        self.status.count_callback();
        let chunk = self.scratch.len();
        let mut frames_done: u64 = 0;
        for part in data.chunks(chunk) {
            let n = part.len();
            for (o, &x) in self.scratch[..n].iter_mut().zip(part) {
                *o = <f32 as cp::FromSample<T>>::from_sample_(x);
            }
            let shifted = InputTimestamp {
                capture_ns: ts.capture_ns + frames_to_ns(frames_done, self.rate_hz),
                ..ts
            };
            self.cb.process(&self.scratch[..n], self.channels, shifted);
            frames_done += (n / self.channels) as u64;
        }
    }

    fn run(&mut self, data: &cp::Data, info: &cp::InputCallbackInfo) {
        let t = info.timestamp();
        let ts = InputTimestamp {
            // The one permitted clock read of this callback (ADR-002 §2).
            now_ns: app_now_ns(),
            callback_ns: ns(t.callback),
            capture_ns: ns(t.capture),
        };
        use cp::SampleFormat as F;
        match data.sample_format() {
            F::F32 => {
                if let Some(s) = data.as_slice::<f32>() {
                    self.run_f32(s, ts);
                }
            }
            F::I32 => self.conv(data.as_slice::<i32>(), ts),
            F::I24 => self.conv(data.as_slice::<cp::I24>(), ts),
            F::F64 => self.conv(data.as_slice::<f64>(), ts),
            F::I16 => self.conv(data.as_slice::<i16>(), ts),
            F::U16 => self.conv(data.as_slice::<u16>(), ts),
            F::I8 => self.conv(data.as_slice::<i8>(), ts),
            F::U8 => self.conv(data.as_slice::<u8>(), ts),
            _ => {}
        }
    }

    #[inline]
    fn conv<T: Copy>(&mut self, data: Option<&[T]>, ts: InputTimestamp)
    where
        f32: cp::FromSample<T>,
    {
        if let Some(d) = data {
            self.run_converted(d, ts);
        }
    }
}

/// RT side of an output stream: the user callback fills f32, converted to the device format.
pub(crate) struct OutputWrap {
    cb: Box<dyn OutputCallback>,
    scratch: Vec<f32>,
    channels: usize,
    rate_hz: u32,
    status: Arc<StreamStatus>,
}

impl OutputWrap {
    /// `scratch_frames`: conversion buffer (frames); callbacks up to it make exactly one call.
    pub(crate) fn new(
        cb: Box<dyn OutputCallback>,
        channels: usize,
        rate_hz: u32,
        scratch_frames: usize,
        status: Arc<StreamStatus>,
    ) -> Self {
        Self {
            cb,
            scratch: vec![0.0; scratch_frames.max(1) * channels.max(1)],
            channels: channels.max(1),
            rate_hz,
            status,
        }
    }

    #[inline]
    pub(crate) fn run_f32(&mut self, data: &mut [f32], ts: OutputTimestamp) {
        self.status.count_callback();
        data.fill(0.0);
        self.cb.process(data, self.channels, ts);
    }

    /// Renders into the scratch buffer chunk by chunk and converts (RT-safe).
    pub(crate) fn run_converted<T>(&mut self, data: &mut [T], ts: OutputTimestamp)
    where
        T: Copy + cp::FromSample<f32>,
    {
        self.status.count_callback();
        let chunk = self.scratch.len();
        let mut frames_done: u64 = 0;
        for part in data.chunks_mut(chunk) {
            let n = part.len();
            let s = &mut self.scratch[..n];
            s.fill(0.0);
            let shifted = OutputTimestamp {
                playback_ns: ts.playback_ns + frames_to_ns(frames_done, self.rate_hz),
                ..ts
            };
            self.cb.process(s, self.channels, shifted);
            for (o, &x) in part.iter_mut().zip(s.iter()) {
                *o = <T as cp::FromSample<f32>>::from_sample_(x);
            }
            frames_done += (n / self.channels) as u64;
        }
    }

    fn run(&mut self, data: &mut cp::Data, info: &cp::OutputCallbackInfo) {
        let t = info.timestamp();
        let ts = OutputTimestamp {
            // The one permitted clock read of this callback (ADR-002 §2).
            now_ns: app_now_ns(),
            callback_ns: ns(t.callback),
            playback_ns: ns(t.playback),
        };
        use cp::SampleFormat as F;
        match data.sample_format() {
            F::F32 => {
                if let Some(s) = data.as_slice_mut::<f32>() {
                    self.run_f32(s, ts);
                }
            }
            F::I32 => self.conv(data.as_slice_mut::<i32>(), ts),
            F::I24 => self.conv(data.as_slice_mut::<cp::I24>(), ts),
            F::F64 => self.conv(data.as_slice_mut::<f64>(), ts),
            F::I16 => self.conv(data.as_slice_mut::<i16>(), ts),
            F::U16 => self.conv(data.as_slice_mut::<u16>(), ts),
            F::I8 => self.conv(data.as_slice_mut::<i8>(), ts),
            F::U8 => self.conv(data.as_slice_mut::<u8>(), ts),
            _ => {}
        }
    }

    #[inline]
    fn conv<T>(&mut self, data: Option<&mut [T]>, ts: OutputTimestamp)
    where
        T: Copy + cp::FromSample<f32>,
    {
        if let Some(d) = data {
            self.run_converted(d, ts);
        }
    }
}

enum Callback {
    Input(Box<dyn InputCallback>),
    Output(Box<dyn OutputCallback>),
}

impl CpalBackend {
    /// A backend over every cpal host compiled in. Fixes the app-clock epoch if not yet set.
    pub fn new() -> Self {
        let _ = app_epoch();
        Self {
            inner: Mutex::new(Inner::default()),
        }
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Takes the cached host (or creates one). PipeWire is always created fresh; so is JACK for
    /// a fresh enumeration.
    fn take_host(&self, host: HostId, fresh: bool) -> Result<cp::Host, BackendError> {
        let reuse = !matches!(host, HostId::PipeWire) && !(fresh && host == HostId::Jack);
        if reuse && let Some(h) = self.lock().hosts.remove(&host) {
            return Ok(h);
        }
        let id = theirs(host).ok_or(BackendError::HostUnavailable(host))?;
        cp::host_from_id(id).map_err(|e| map_err(&e, host, None))
    }

    fn put_host(&self, host: HostId, h: cp::Host) {
        if host != HostId::PipeWire {
            self.lock().hosts.insert(host, h);
        }
    }

    /// Reads one direction: `None` = unsupported, `Unavailable` = read failed.
    fn read_dir(
        host: HostId,
        configs: Result<Vec<cp::SupportedStreamConfigRange>, cp::Error>,
        default: Option<cp::SupportedStreamConfig>,
    ) -> Option<CapsState> {
        match configs {
            Err(e) if e.kind() == cp::ErrorKind::UnsupportedOperation => None,
            Err(_) => Some(CapsState::Unavailable),
            Ok(c) if c.is_empty() => None,
            // No format we can convert: unsupported for us.
            Ok(c) => caps_from(&c, default, is_plug_layer(host, &c)).map(CapsState::Known),
        }
    }

    fn wanted_dirs(dir: cp::DeviceDirection) -> (bool, bool) {
        (
            !matches!(dir, cp::DeviceDirection::Output),
            !matches!(dir, cp::DeviceDirection::Input),
        )
    }

    fn query_caps(host: HostId, device: &cp::Device, dir: cp::DeviceDirection) -> CachedCaps {
        let (wants_in, wants_out) = Self::wanted_dirs(dir);
        let input = wants_in
            .then(|| {
                let configs = device.supported_input_configs().map(|i| i.collect());
                Self::read_dir(host, configs, device.default_input_config().ok())
            })
            .flatten();
        let output = wants_out
            .then(|| {
                let configs = device.supported_output_configs().map(|i| i.collect());
                Self::read_dir(host, configs, device.default_output_config().ok())
            })
            .flatten();
        CachedCaps { input, output }
    }

    /// Hosts whose capabilities come for free (no device is opened to read them).
    fn cheap_caps(host: HostId) -> bool {
        matches!(host, HostId::PipeWire | HostId::Jack)
    }

    fn enumerate_host(
        &self,
        host: HostId,
        h: &cp::Host,
        mode: Enumerate,
    ) -> Result<DeviceSnapshot, BackendError> {
        struct Raw {
            id: cp::DeviceId,
            base: String,
            dir: cp::DeviceDirection,
            device: cp::Device,
        }
        let mut raw: Vec<Raw> = Vec::new();
        let mut seen: HashSet<cp::DeviceId> = HashSet::new();
        let listed = h.devices().map_err(|e| map_err(&e, host, None))?;
        let defaults = [h.default_input_device(), h.default_output_device()];
        for d in listed.chain(defaults.iter().flatten().cloned()) {
            let Ok(id) = d.id() else { continue };
            if !seen.insert(id.clone()) {
                continue;
            }
            let desc = d.description().ok();
            let base = desc
                .as_ref()
                .map(|x| x.name().trim().to_owned())
                .filter(|n| !n.is_empty())
                .unwrap_or_else(|| id.id().to_owned());
            let dir = desc.map_or(cp::DeviceDirection::Unknown, |x| x.direction());
            raw.push(Raw {
                id,
                base,
                dir,
                device: d,
            });
        }
        let names = unique_names(
            host,
            &raw.iter()
                .map(|r| (r.base.clone(), r.id.id().to_owned()))
                .collect::<Vec<_>>(),
        );

        let cached: HashMap<String, CachedCaps> = {
            let inner = self.lock();
            raw.iter()
                .filter_map(|r| {
                    let k = r.id.id().to_owned();
                    inner.caps.get(&(host, k.clone())).map(|c| (k, c.clone()))
                })
                .collect()
        };
        let mut devices = Vec::with_capacity(raw.len());
        // Devices whose capabilities were read in this pass (their cache entry is replaced).
        let mut read: HashSet<String> = HashSet::new();
        for (r, name) in raw.iter().zip(&names) {
            let id = r.id.id().to_owned();
            let prev = cached.get(&id);
            let caps = match (mode, prev) {
                (Enumerate::Quick | Enumerate::Cached, Some(p)) => p.clone(),
                (Enumerate::Quick, None) if !Self::cheap_caps(host) => {
                    let (i, o) = Self::wanted_dirs(r.dir);
                    CachedCaps {
                        input: i.then_some(CapsState::Pending),
                        output: o.then_some(CapsState::Pending),
                    }
                }
                (_, prev) => {
                    let q = Self::query_caps(host, &r.device, r.dir);
                    read.insert(id.clone());
                    // A failed read (busy: we may be streaming on it) keeps known capabilities.
                    let keep = |new: Option<CapsState>, old: Option<&CapsState>| match (new, old) {
                        (Some(CapsState::Unavailable), Some(k @ CapsState::Known(_))) => {
                            Some(k.clone())
                        }
                        (new, _) => new,
                    };
                    CachedCaps {
                        input: keep(q.input, prev.and_then(|p| p.input.as_ref())),
                        output: keep(q.output, prev.and_then(|p| p.output.as_ref())),
                    }
                }
            };
            devices.push((id, caps, name.clone(), r));
        }
        let infos: Vec<DeviceInfo> = devices
            .iter()
            .map(|(id, caps, name, r)| DeviceInfo {
                name: name.clone(),
                base_name: r.base.clone(),
                id: id.clone(),
                input: caps.input.clone(),
                output: caps.output.clone(),
                system_default: is_system_default(host, id),
                input_is_monitor: is_monitor_input(host, id, r.dir),
            })
            .collect();
        let name_of = |d: &Option<cp::Device>| {
            let id = d.as_ref()?.id().ok()?;
            raw.iter()
                .position(|r| r.id == id)
                .map(|p| names[p].clone())
        };
        let mut snap = DeviceSnapshot {
            default_input: name_of(&defaults[0]),
            default_output: name_of(&defaults[1]),
            devices: infos,
        };
        snap.normalize();

        let mut inner = self.lock();
        let present: HashSet<&str> = devices.iter().map(|(id, ..)| id.as_str()).collect();
        inner
            .caps
            .retain(|(h, id), _| *h != host || present.contains(id.as_str()));
        for (id, caps, ..) in &devices {
            if read.contains(id) {
                inner.caps.insert((host, id.clone()), caps.clone());
            }
        }
        inner.ids.retain(|(h, _), _| *h != host);
        for r in &raw {
            inner.ids.insert((host, r.id.id().to_owned()), r.id.clone());
        }
        inner.snapshots.insert(host, snap.clone());
        Ok(snap)
    }

    /// The cpal id of the device `name` refers to (see [`DeviceSnapshot::lookup`]).
    fn resolve_id(&self, host: HostId, name: &str, dir: Direction) -> Option<cp::DeviceId> {
        let inner = self.lock();
        let info = inner.snapshots.get(&host)?.lookup(name, dir)?;
        inner.ids.get(&(host, info.id.clone())).cloned()
    }

    fn open(
        &self,
        req: &StreamRequest,
        dir: Direction,
        cb: Callback,
    ) -> Result<StreamHandle, BackendError> {
        let host = req.device.host;
        let h = self.take_host(host, false)?;
        let result = (|| {
            let id = match self.resolve_id(host, &req.device.name, dir) {
                Some(id) => id,
                None => {
                    self.enumerate_host(host, &h, Enumerate::Quick)?;
                    self.resolve_id(host, &req.device.name, dir)
                        .ok_or_else(|| BackendError::DeviceNotFound(req.device.clone()))?
                }
            };
            let device = h
                .device_by_id(&id)
                .ok_or_else(|| BackendError::DeviceNotFound(req.device.clone()))?;
            self.build(req, dir, cb, &device)
        })();
        self.put_host(host, h);
        result
    }

    fn build(
        &self,
        req: &StreamRequest,
        dir: Direction,
        cb: Callback,
        device: &cp::Device,
    ) -> Result<StreamHandle, BackendError> {
        let key = &req.device;
        let err = |e: cp::Error| map_err(&e, key.host, Some(key));
        let configs: Vec<cp::SupportedStreamConfigRange> = match dir {
            Direction::Input => device.supported_input_configs().map_err(err)?.collect(),
            Direction::Output => device.supported_output_configs().map_err(err)?.collect(),
        };
        if configs.is_empty() {
            return Err(BackendError::WrongDirection(key.clone(), dir));
        }
        let rate = req.sample_rate_hz;
        let plug = is_plug_layer(key.host, &configs);
        let wanted = req.channels.unwrap_or_else(|| match dir {
            // Plug-layer devices offer 1…64 channels; open the device's own count.
            Direction::Input if plug => device
                .default_input_config()
                .map(|c| c.channels())
                .unwrap_or(2),
            Direction::Input => configs
                .iter()
                .filter(|c| c.contains_rate(rate))
                .map(|c| c.channels())
                .max()
                .unwrap_or(1),
            Direction::Output => device
                .default_output_config()
                .map(|c| c.channels())
                .unwrap_or(2),
        });
        let picked = pick_config(&configs, rate, wanted, req.buffer).ok_or_else(|| {
            BackendError::UnsupportedConfig(format!(
                "{rate} Hz / {wanted} ch not supported by {key}"
            ))
        })?;
        let (channels, format, applied) = (picked.channels, picked.format, picked.applied);
        let config = cp::StreamConfig {
            channels,
            sample_rate: rate,
            buffer_size: picked.size,
        };
        // f32 needs no conversion buffer.
        let scratch = if format == cp::SampleFormat::F32 {
            1
        } else {
            scratch_frames(picked.size, picked.range)
        };
        let status = StreamStatus::new();
        let err_status = status.clone();
        // cpal passes an owned `Error` that may carry a heap-allocated message; it is dropped here,
        // on whichever thread cpal reports errors from (possibly its audio thread). Accepted: that
        // memory was allocated by cpal in the same error path, and our side only touches atomics.
        let on_error = move |e: cp::Error| err_status.report(classify(e.kind()));
        let ch = usize::from(channels);
        let stream = match cb {
            Callback::Input(cb) => {
                let mut w = InputWrap::new(cb, ch, rate, scratch, status.clone());
                device.build_input_stream_raw(
                    config,
                    format,
                    move |data: &cp::Data, info: &cp::InputCallbackInfo| w.run(data, info),
                    on_error,
                    None,
                )
            }
            Callback::Output(cb) => {
                let mut w = OutputWrap::new(cb, ch, rate, scratch, status.clone());
                device.build_output_stream_raw(
                    config,
                    format,
                    move |data: &mut cp::Data, info: &cp::OutputCallbackInfo| w.run(data, info),
                    on_error,
                    None,
                )
            }
        }
        .map_err(err)?;
        stream.play().map_err(err)?;
        let nominal_frames = stream.buffer_size().ok();
        let id = {
            let mut inner = self.lock();
            inner.next_stream += 1;
            StreamId(inner.next_stream)
        };
        let info = StreamInfo {
            id,
            device: key.clone(),
            direction: dir,
            channels,
            sample_rate_hz: rate,
            buffer: applied,
            nominal_frames,
        };
        Ok(StreamHandle::new(info, status, Box::new(stream)))
    }
}

impl Backend for CpalBackend {
    fn hosts(&self) -> Vec<HostId> {
        let available = cp::available_hosts();
        HostId::ALL
            .into_iter()
            .filter(|h| theirs(*h).is_some_and(|c| available.contains(&c)))
            .filter(|&h| {
                // JACK always claims availability; it is usable only if a client connects.
                if h != HostId::Jack {
                    return true;
                }
                match self.take_host(h, true) {
                    Ok(host) => {
                        let ok = host.devices().is_ok_and(|mut d| d.next().is_some());
                        self.put_host(h, host);
                        ok
                    }
                    Err(_) => false,
                }
            })
            .collect()
    }

    fn enumerate(&self, host: HostId, mode: Enumerate) -> Result<DeviceSnapshot, BackendError> {
        let h = self.take_host(host, mode == Enumerate::Fresh)?;
        let result = self.enumerate_host(host, &h, mode);
        self.put_host(host, h);
        result
    }

    fn open_input(
        &self,
        request: &StreamRequest,
        callback: Box<dyn InputCallback>,
    ) -> Result<StreamHandle, BackendError> {
        self.open(request, Direction::Input, Callback::Input(callback))
    }

    fn open_output(
        &self,
        request: &StreamRequest,
        callback: Box<dyn OutputCallback>,
    ) -> Result<StreamHandle, BackendError> {
        self.open(request, Direction::Output, Callback::Output(callback))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use vox_module_api::test_util::no_alloc;

    fn range(ch: u16, fmt: cp::SampleFormat, min: u32, max: u32) -> cp::SupportedStreamConfigRange {
        cp::SupportedStreamConfigRange::new(
            ch,
            min,
            max,
            cp::SupportedBufferSize::Range { min: 64, max: 4096 },
            fmt,
        )
    }

    #[test]
    fn host_ids_map_both_ways() {
        for c in cp::ALL_HOSTS {
            if let Some(h) = ours(*c) {
                assert_eq!(theirs(h), Some(*c));
            }
        }
        #[cfg(target_os = "linux")]
        {
            assert!(theirs(HostId::Alsa).is_some());
            assert!(theirs(HostId::PipeWire).is_some());
            assert!(theirs(HostId::Jack).is_some());
        }
    }

    #[test]
    fn device_names_are_unique_and_stable() {
        let e = |n: &str, id: &str| (n.to_owned(), id.to_owned());
        // ALSA: always suffixed with the PCM id, so a key never changes between passes.
        let names = unique_names(
            HostId::Alsa,
            &[
                e("HDA Intel PCH", "hw:CARD=0,DEV=0"),
                e("HDA Intel PCH", "plughw:CARD=0,DEV=0"),
                e("USB Mic", "hw:CARD=1,DEV=0"),
            ],
        );
        assert_eq!(
            names,
            vec![
                "HDA Intel PCH (hw:CARD=0,DEV=0)",
                "HDA Intel PCH (plughw:CARD=0,DEV=0)",
                "USB Mic (hw:CARD=1,DEV=0)"
            ]
        );
        // PipeWire: plain unless two nodes share a description, then the node name is added.
        let names = unique_names(
            HostId::PipeWire,
            &[
                e("USB Mic Mono", "alsa_input.usb-A"),
                e("USB Mic Mono", "alsa_input.usb-B"),
                e("Speakers", "alsa_output.pci"),
            ],
        );
        assert_eq!(
            names,
            vec![
                "USB Mic Mono (alsa_input.usb-A)",
                "USB Mic Mono (alsa_input.usb-B)",
                "Speakers"
            ]
        );
    }

    #[test]
    fn default_and_monitor_flags() {
        use cp::DeviceDirection as D;
        assert!(is_system_default(HostId::PipeWire, "output_default"));
        assert!(is_system_default(HostId::PipeWire, "sink_default"));
        assert!(is_system_default(HostId::Alsa, "default"));
        assert!(!is_system_default(HostId::Alsa, "sysdefault:CARD=0"));
        assert!(!is_system_default(HostId::PipeWire, "alsa_output.pci"));
        assert!(is_monitor_input(
            HostId::PipeWire,
            "sink_default",
            D::Duplex
        ));
        assert!(is_monitor_input(
            HostId::PipeWire,
            "alsa_output.pci-0000.analog-stereo",
            D::Duplex
        ));
        assert!(!is_monitor_input(
            HostId::PipeWire,
            "alsa_input.usb",
            D::Input
        ));
        assert!(!is_monitor_input(
            HostId::PipeWire,
            "input_default",
            D::Input
        ));
        assert!(!is_monitor_input(
            HostId::PipeWire,
            "alsa_duplex.x",
            D::Duplex
        ));
        assert!(!is_monitor_input(
            HostId::Alsa,
            "hw:CARD=0,DEV=0",
            D::Duplex
        ));
    }

    #[test]
    fn plug_layer_devices_use_their_default_channel_count() {
        use cp::SampleFormat as F;
        assert!(
            is_plug_layer(HostId::Jack, &[]),
            "JACK's 1…64 ports are synthetic"
        );
        assert!(!is_plug_layer(
            HostId::Alsa,
            &[range(2, F::I16, 44_100, 48_000)]
        ));
        let plug = [
            range(64, F::F32, 4_000, u32::MAX),
            range(1, F::I16, 4_000, u32::MAX),
        ];
        assert!(is_plug_layer(HostId::Alsa, &plug));
        assert!(!is_plug_layer(HostId::PipeWire, &plug));
        let default = cp::SupportedStreamConfig::new(
            2,
            48_000,
            cp::SupportedBufferSize::Range { min: 64, max: 4096 },
            F::F32,
        );
        let c = caps_from(&plug, Some(default), true).unwrap();
        assert_eq!((c.max_channels, c.default_channels), (2, 2));
        let c = caps_from(&plug, Some(default), false).unwrap();
        assert_eq!(c.max_channels, 64);
    }

    #[test]
    fn scratch_is_sized_from_the_opened_buffer() {
        use cp::{BufferSize as B, SupportedBufferSize as R};
        let r = |min, max| R::Range { min, max };
        assert_eq!(scratch_frames(B::Fixed(256), r(64, 8192)), 1024);
        assert_eq!(scratch_frames(B::Fixed(4096), r(64, 8192)), 4096);
        assert_eq!(scratch_frames(B::Default, r(64, 8192)), 8192);
        assert_eq!(scratch_frames(B::Default, r(1, 268_435_455)), 16_384);
        assert_eq!(scratch_frames(B::Default, R::Unknown), 16_384);
    }

    #[test]
    fn error_classes() {
        use cp::ErrorKind as K;
        assert_eq!(classify(K::DeviceNotAvailable), ErrorClass::DeviceLost);
        assert_eq!(classify(K::StreamInvalidated), ErrorClass::DeviceLost);
        assert_eq!(classify(K::HostUnavailable), ErrorClass::DeviceLost);
        assert_eq!(classify(K::Xrun), ErrorClass::Xrun);
        assert_eq!(classify(K::DeviceChanged), ErrorClass::Benign);
        assert_eq!(classify(K::BackendError), ErrorClass::Other);
    }

    #[test]
    fn caps_and_config_selection() {
        use cp::SampleFormat as F;
        let configs = [
            range(2, F::I16, 44_100, 48_000),
            range(2, F::F32, 44_100, 96_000),
            range(4, F::I32, 48_000, 48_000),
            range(2, F::DsdU8, 1, 1_000_000),
        ];
        let c = caps_from(&configs, None, false).unwrap();
        assert_eq!(c.max_channels, 4);
        assert_eq!(c.default_channels, 2);
        assert_eq!(c.default_rate_hz, 48_000);
        assert_eq!(
            c.rates,
            vec![RateRange {
                min_hz: 44_100,
                max_hz: 96_000
            }]
        );
        assert_eq!(
            c.buffer_range,
            Some(BufferRange {
                min_frames: 64,
                max_frames: 4096
            })
        );
        let pick = pick_config(&configs, 48_000, 2, BufferRequest::Frames(8192)).unwrap();
        assert_eq!(pick.channels, 2);
        assert_eq!(pick.format, F::F32);
        assert_eq!(pick.size, cp::BufferSize::Fixed(4096));
        assert_eq!(pick.applied, BufferRequest::Frames(4096));
        let pick = pick_config(&configs, 48_000, 4, BufferRequest::Auto).unwrap();
        assert_eq!(
            (pick.channels, pick.format, pick.size),
            (4, F::I32, cp::BufferSize::Default)
        );
        assert!(pick_config(&configs, 192_000, 2, BufferRequest::Auto).is_none());
    }

    #[test]
    fn input_conversion_is_one_call_per_callback_and_allocation_free() {
        let sum = Arc::new(AtomicU64::new(0));
        let calls = Arc::new(AtomicU64::new(0));
        let now = Arc::new(AtomicU64::new(0));
        let (s2, c2, n2) = (sum.clone(), calls.clone(), now.clone());
        let cb = move |d: &[f32], ch: usize, ts: InputTimestamp| {
            c2.fetch_add(1, Ordering::Relaxed);
            n2.store(ts.now_ns, Ordering::Relaxed);
            let v: f32 = d.chunks_exact(ch).map(|f| f[1]).sum();
            s2.fetch_add(v as u64, Ordering::Relaxed);
        };
        let status = StreamStatus::new();
        let mut w = InputWrap::new(Box::new(cb), 2, 48_000, 4096, status.clone());
        // 3000 frames: ch0 = 0, ch1 = +0.5 full scale (16384 / 32768).
        let data: Vec<i16> = (0..3000).flat_map(|_| [0i16, 16_384]).collect();
        let ts = InputTimestamp {
            now_ns: 77,
            callback_ns: 0,
            capture_ns: 0,
        };
        no_alloc(|| w.run_converted(&data, ts)).expect("conversion allocated");
        assert_eq!(
            calls.load(Ordering::Relaxed),
            1,
            "one device callback, one call"
        );
        assert_eq!(sum.load(Ordering::Relaxed), 1500);
        assert_eq!(now.load(Ordering::Relaxed), 77);
        assert_eq!(status.callback_count(), 1);
        // Larger than the scratch (only past the opened buffer size): split, same clock read.
        let mut small = InputWrap::new(
            Box::new(|_: &[f32], _: usize, _: InputTimestamp| {}),
            2,
            48_000,
            1024,
            StreamStatus::new(),
        );
        no_alloc(|| small.run_converted(&data, ts)).expect("conversion allocated");
    }

    #[test]
    fn output_conversion_is_chunked_and_allocation_free() {
        let cb = |d: &mut [f32], _ch: usize, _ts: OutputTimestamp| d.fill(0.5);
        let status = StreamStatus::new();
        let mut w = OutputWrap::new(Box::new(cb), 2, 48_000, 1024, status);
        let mut data = vec![0i16; 2 * 2500];
        let ts = OutputTimestamp {
            now_ns: 0,
            callback_ns: 0,
            playback_ns: 0,
        };
        no_alloc(|| w.run_converted(&mut data, ts)).expect("conversion allocated");
        assert!(data.iter().all(|&x| x == 16_384));
        let mut f = vec![1.0f32; 64];
        no_alloc(|| w.run_f32(&mut f, ts)).expect("f32 path allocated");
        assert!(f.iter().all(|&x| x.to_bits() == 0.5f32.to_bits()));
    }
}

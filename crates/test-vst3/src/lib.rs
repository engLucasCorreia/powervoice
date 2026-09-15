//! The in-repo **test VST3 plugin** (T-806): a `cdylib` exporting `GetPluginFactory` (plus the
//! per-OS module entry points), used by the sandbox's VST3 backend tests instead of installed
//! plugins. Its factory (`IPluginFactory2`) offers:
//!
//! | Class | Controller | Buses | Parameters | Latency |
//! |---|---|---|---|---|
//! | [`ID_GAIN`] | separate, connected through connection points | mono | `Gain` ([`PARAM_GAIN`]) | [`LATENCY_SAMPLES`] |
//! | [`ID_GAIN_STEREO`] | combined (the component is its own controller) | **stereo only** (the host's mono shim) | `Gain` | [`LATENCY_SAMPLES`] |
//! | [`ID_LATENCY`] | separate, with an "Advanced" unit | mono | `Gain`, `Latency` ([`PARAM_LATENCY`]) | the `Latency` value |
//!
//! Every class runs the built-in Gain (`vox_modules::Gain`, bit-exact with the in-process
//! module) per channel, then a delay line of its latency.
//!
//! - **Gain:** continuous; normalized `n` ↔ `−60 + 84·n` dB, quantized to 0.001 dB
//!   ([`normalized_to_gain_db`]). Text `"-6.00"`, units `"dB"`.
//! - **Latency:** discrete, [`MAX_LATENCY`] steps = samples, units `"smp"`. When the controller
//!   gets a value that differs from the latency the processor reported at its last activation
//!   (an `IMessage` the processor sends through the connection points, allocated by the host's
//!   `IHostApplication::createInstance`), it calls `restartComponent(kLatencyChanged)`.
//! - **Parameter changes** are sample-accurate points of the `IParameterChanges` queues. A
//!   zero-sample `process` (the host's parameter flush) only records values; they apply, snapped
//!   (no ramp), at the next activation.
//! - **Component state:** [`COMPONENT_STATE_MAGIC`], version `u32` = 1, gain dB `f64`, latency
//!   `u32` (little-endian). **Controller state:** [`CONTROLLER_STATE_MAGIC`], version `u32` = 1,
//!   `ui_scale` `u32` (controller-only data).
//! - A copy whose file name contains [`CRASH_ON_SCAN_MARKER`] aborts in `ModuleEntry` (Linux);
//!   [`HANG_ON_SCAN_MARKER`] never returns from it.

#![allow(non_snake_case)]
// VST3 enum constants are `c_uint` on unix and `c_int` on Windows (`DefaultEnumType`): a cast
// that is a no-op on one platform is needed on the other.
#![allow(clippy::unnecessary_cast)]

use std::ffi::{CStr, c_char, c_void};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};

use vox_module_api::{
    ActivateConfig, ChannelLayout, EventList, Module, OutputEvents, ParamEvent, ProcessMode,
    Transport,
};
use vox_modules::Gain;
use vst3::Steinberg::Vst::*;
use vst3::Steinberg::*;
use vst3::{Class, ComPtr, ComRef, ComWrapper, Interface, uid};

/// Mono gain (separate controller).
pub const ID_GAIN: &str = "50565633544553544741494E00000001";
/// Stereo-only gain (combined component + controller).
pub const ID_GAIN_STEREO: &str = "50565633544553544741494E00000002";
/// Gain with a latency parameter (separate controller).
pub const ID_LATENCY: &str = "50565633544553544C41545900000001";
/// Every audio processor class id, in factory order.
pub const IDS: [&str; 3] = [ID_GAIN, ID_GAIN_STEREO, ID_LATENCY];

const CID_GAIN: TUID = uid(0x5056_5633, 0x5445_5354, 0x4741_494E, 0x0000_0001);
const CID_GAIN_CTRL: TUID = uid(0x5056_5633, 0x5445_5354, 0x4354_524C, 0x0000_0001);
const CID_GAIN_STEREO: TUID = uid(0x5056_5633, 0x5445_5354, 0x4741_494E, 0x0000_0002);
const CID_LATENCY: TUID = uid(0x5056_5633, 0x5445_5354, 0x4C41_5459, 0x0000_0001);
const CID_LATENCY_CTRL: TUID = uid(0x5056_5633, 0x5445_5354, 0x4354_524C, 0x0000_0003);

/// Fixed latency of the gain classes (and the default of `Latency`).
pub const LATENCY_SAMPLES: u32 = 64;
/// Largest `Latency` (its step count).
pub const MAX_LATENCY: u32 = 4096;
/// Parameter id of `Gain`.
pub const PARAM_GAIN: u32 = 0;
/// Parameter id of `Latency`.
pub const PARAM_LATENCY: u32 = 1;
/// `Gain`'s range in dB.
pub const MIN_GAIN_DB: f64 = -60.0;
/// `Gain`'s range in dB.
pub const MAX_GAIN_DB: f64 = 24.0;
/// The unit (group) of `Latency`.
pub const ADVANCED_UNIT: i32 = 1;
/// A bundle whose binary's file name contains this aborts in `ModuleEntry`.
pub const CRASH_ON_SCAN_MARKER: &str = "crash-on-scan";
/// A bundle whose binary's file name contains this never returns from `ModuleEntry`.
pub const HANG_ON_SCAN_MARKER: &str = "hang-on-scan";
/// Component state magic.
pub const COMPONENT_STATE_MAGIC: [u8; 4] = *b"PVT3";
/// Controller state magic.
pub const CONTROLLER_STATE_MAGIC: [u8; 4] = *b"PVC3";
/// The controller-only value its state carries.
pub const UI_SCALE: u32 = 100;
/// The message the processor sends its controller at activation.
const ACTIVE_LATENCY_MESSAGE: &CStr = c"activeLatency";
const SAMPLES_ATTR: &CStr = c"samples";

/// `Gain` dB → normalized.
pub fn gain_db_to_normalized(db: f64) -> f64 {
    ((db - MIN_GAIN_DB) / (MAX_GAIN_DB - MIN_GAIN_DB)).clamp(0.0, 1.0)
}

/// Normalized → `Gain` dB, quantized to 0.001 dB (so round trips of the tests' values are exact).
pub fn normalized_to_gain_db(n: f64) -> f64 {
    let db = MIN_GAIN_DB + (MAX_GAIN_DB - MIN_GAIN_DB) * n.clamp(0.0, 1.0);
    (db * 1000.0).round() / 1000.0 + 0.0
}

/// `Latency` samples → normalized (the SDK's discrete mapping: `value / stepCount`).
pub fn latency_to_normalized(samples: u32) -> f64 {
    f64::from(samples.min(MAX_LATENCY)) / f64::from(MAX_LATENCY)
}

/// Normalized → `Latency` samples (the SDK's `min(stepCount, n · (stepCount + 1))`).
pub fn normalized_to_latency(n: f64) -> u32 {
    ((n.clamp(0.0, 1.0) * f64::from(MAX_LATENCY + 1)).floor() as u32).min(MAX_LATENCY)
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

fn copy_cstring(src: &str, dst: &mut [c_char]) {
    let n = src.len().min(dst.len().saturating_sub(1));
    for (d, s) in dst.iter_mut().zip(&src.as_bytes()[..n]) {
        *d = *s as c_char;
    }
    if let Some(end) = dst.get_mut(n) {
        *end = 0;
    }
}

fn copy_wstring(src: &str, dst: &mut [TChar]) {
    let mut len = 0;
    for (d, s) in dst.iter_mut().zip(src.encode_utf16()) {
        *d = s as TChar;
        len += 1;
    }
    let end = len.min(dst.len().saturating_sub(1));
    if let Some(e) = dst.get_mut(end) {
        *e = 0;
    }
}

/// # Safety
/// `s` is a NUL-terminated UTF-16 string.
unsafe fn read_wstring(s: *const TChar) -> String {
    let mut len = 0;
    // SAFETY: caller's contract (NUL-terminated).
    while unsafe { *s.add(len) } != 0 {
        len += 1;
    }
    // SAFETY: `len` characters were just read.
    let units = unsafe { std::slice::from_raw_parts(s.cast::<u16>(), len) };
    String::from_utf16_lossy(units)
}

/// Writes `bytes` to the host's stream.
///
/// # Safety
/// `stream` is null or a live `IBStream`.
unsafe fn write_all(stream: *mut IBStream, bytes: &[u8]) -> bool {
    // SAFETY: caller's contract.
    let Some(s) = (unsafe { ComRef::from_raw(stream) }) else {
        return false;
    };
    let mut rest = bytes;
    while !rest.is_empty() {
        let mut written = 0;
        // SAFETY: a live stream; `rest` is readable for its length.
        let r = unsafe {
            s.write(
                rest.as_ptr() as *mut c_void,
                rest.len() as i32,
                &mut written,
            )
        };
        if r != kResultOk || written <= 0 {
            return false;
        }
        rest = &rest[(written as usize).min(rest.len())..];
    }
    true
}

/// Reads the host's stream to its end, in small pieces (partial reads are part of the contract).
///
/// # Safety
/// `stream` is null or a live `IBStream`.
unsafe fn read_all(stream: *mut IBStream) -> Option<Vec<u8>> {
    // SAFETY: caller's contract.
    let s = unsafe { ComRef::from_raw(stream) }?;
    let mut out = Vec::new();
    let mut chunk = [0u8; 7];
    loop {
        let mut read = 0;
        // SAFETY: a live stream; `chunk` is writable for its length.
        let r = unsafe { s.read(chunk.as_mut_ptr().cast(), chunk.len() as i32, &mut read) };
        if r != kResultOk || read <= 0 {
            return Some(out);
        }
        out.extend_from_slice(&chunk[..(read as usize).min(chunk.len())]);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Mono,
    Stereo,
    Latency,
}

impl Kind {
    fn channels(self) -> usize {
        if self == Kind::Stereo { 2 } else { 1 }
    }

    fn arrangement(self) -> SpeakerArrangement {
        if self == Kind::Stereo {
            SpeakerArr::kStereo
        } else {
            SpeakerArr::kMono
        }
    }

    fn param_count(self) -> i32 {
        if self == Kind::Latency { 2 } else { 1 }
    }
}

// --- The processor ---------------------------------------------------------------------------

struct Audio {
    gains: Vec<Gain>,
    events: EventList,
    out: OutputEvents,
    /// Per channel: a ring of the active latency.
    delay: Vec<Vec<f32>>,
    delay_pos: usize,
    scratch: Vec<f32>,
    steady: u64,
}

impl Audio {
    fn new(kind: Kind) -> Self {
        Self {
            gains: (0..kind.channels()).map(|_| Gain::new()).collect(),
            events: EventList::with_capacity(vox_module_api::DEFAULT_EVENT_CAPACITY),
            out: OutputEvents::with_capacity(vox_module_api::DEFAULT_EVENT_CAPACITY),
            delay: Vec::new(),
            delay_pos: 0,
            scratch: Vec::new(),
            steady: 0,
        }
    }
}

#[derive(Clone, Copy)]
struct Setup {
    sample_rate: f64,
    max_block: u32,
    offline: bool,
}

/// The audio processor component. `COMBINED`: it is its own edit controller (`ID_GAIN_STEREO`).
struct Effect<const COMBINED: bool> {
    kind: Kind,
    /// `Gain` in dB as last set (bits of an `f64`).
    gain_db: AtomicU64,
    /// `Latency` as last set (applies at the next activation).
    latency_param: AtomicU32,
    /// The latency of the current activation.
    active_latency: AtomicU32,
    setup: Mutex<Setup>,
    audio: Mutex<Audio>,
    host: Mutex<Option<ComPtr<IHostApplication>>>,
    peer: Mutex<Option<ComPtr<IConnectionPoint>>>,
    /// The controller half of a combined component.
    ctrl: ControllerCore,
}

impl Class for Effect<false> {
    type Interfaces = (IComponent, IAudioProcessor, IConnectionPoint);
}

impl Class for Effect<true> {
    type Interfaces = (IComponent, IAudioProcessor, IEditController);
}

impl<const C: bool> Effect<C> {
    fn new(kind: Kind) -> Self {
        Self {
            kind,
            gain_db: AtomicU64::new(0f64.to_bits()),
            latency_param: AtomicU32::new(LATENCY_SAMPLES),
            active_latency: AtomicU32::new(LATENCY_SAMPLES),
            setup: Mutex::new(Setup {
                sample_rate: 48_000.0,
                max_block: 1024,
                offline: false,
            }),
            audio: Mutex::new(Audio::new(kind)),
            host: Mutex::new(None),
            peer: Mutex::new(None),
            ctrl: ControllerCore::new(kind),
        }
    }

    fn gain_db(&self) -> f64 {
        f64::from_bits(self.gain_db.load(Ordering::Acquire))
    }

    fn latency(&self) -> u32 {
        match self.kind {
            Kind::Latency => self.latency_param.load(Ordering::Acquire),
            _ => LATENCY_SAMPLES,
        }
    }

    fn state_bytes(&self) -> Vec<u8> {
        let mut b = Vec::with_capacity(20);
        b.extend_from_slice(&COMPONENT_STATE_MAGIC);
        b.extend_from_slice(&1u32.to_le_bytes());
        b.extend_from_slice(&self.gain_db().to_le_bytes());
        b.extend_from_slice(&self.latency_param.load(Ordering::Acquire).to_le_bytes());
        b
    }

    /// Tells the controller (through the connection) which latency this activation reports.
    fn announce_latency(&self, samples: u32) {
        let host = lock(&self.host).clone();
        let peer = lock(&self.peer).clone();
        let (Some(host), Some(peer)) = (host, peer) else {
            return;
        };
        let mut obj: *mut c_void = std::ptr::null_mut();
        let iid = IMessage::IID;
        // SAFETY: the host's application object; `obj` receives an owned reference.
        let r = unsafe {
            host.createInstance(
                iid.as_ptr() as *mut TUID,
                iid.as_ptr() as *mut TUID,
                &mut obj,
            )
        };
        // SAFETY: on success `obj` is an `IMessage` whose reference we now own.
        let Some(msg) = (r == kResultOk)
            .then(|| unsafe { ComPtr::from_raw(obj.cast::<IMessage>()) })
            .flatten()
        else {
            return;
        };
        // SAFETY: a live message; NUL-terminated ids; the peer is the connected controller.
        unsafe {
            msg.setMessageID(ACTIVE_LATENCY_MESSAGE.as_ptr());
            if let Some(attrs) = ComRef::from_raw(msg.getAttributes()) {
                attrs.setInt(SAMPLES_ATTR.as_ptr(), i64::from(samples));
            }
            peer.notify(msg.as_ptr());
        }
    }
}

/// Parses a component state.
fn parse_component_state(bytes: &[u8]) -> Option<(f64, u32)> {
    if bytes.len() != 20 || bytes[..4] != COMPONENT_STATE_MAGIC || bytes[4..8] != 1u32.to_le_bytes()
    {
        return None;
    }
    let gain = f64::from_le_bytes(bytes[8..16].try_into().ok()?);
    let latency = u32::from_le_bytes(bytes[16..20].try_into().ok()?);
    Some((gain, latency.min(MAX_LATENCY)))
}

impl<const C: bool> IPluginBaseTrait for Effect<C> {
    unsafe fn initialize(&self, context: *mut FUnknown) -> tresult {
        // SAFETY: the host's context object (or null), valid for the call; we keep a reference.
        let host = unsafe { ComRef::from_raw(context) }.and_then(|c| c.cast::<IHostApplication>());
        *lock(&self.host) = host;
        kResultOk
    }

    unsafe fn terminate(&self) -> tresult {
        *lock(&self.host) = None;
        *lock(&self.peer) = None;
        *lock(&self.ctrl.handler) = None;
        kResultOk
    }
}

impl<const C: bool> IComponentTrait for Effect<C> {
    unsafe fn getControllerClassId(&self, class_id: *mut TUID) -> tresult {
        let cid = match self.kind {
            _ if C => return kNotImplemented,
            Kind::Latency => CID_LATENCY_CTRL,
            _ => CID_GAIN_CTRL,
        };
        // SAFETY: the host's writable TUID.
        unsafe { *class_id = cid };
        kResultOk
    }

    unsafe fn setIoMode(&self, _mode: IoMode) -> tresult {
        kResultOk
    }

    unsafe fn getBusCount(&self, media_type: MediaType, _dir: BusDirection) -> int32 {
        i32::from(media_type as MediaTypes == MediaTypes_::kAudio)
    }

    unsafe fn getBusInfo(
        &self,
        media_type: MediaType,
        dir: BusDirection,
        index: int32,
        bus: *mut BusInfo,
    ) -> tresult {
        if media_type as MediaTypes != MediaTypes_::kAudio || index != 0 || bus.is_null() {
            return kInvalidArgument;
        }
        // SAFETY: the host's writable bus info.
        let bus = unsafe { &mut *bus };
        bus.mediaType = media_type;
        bus.direction = dir;
        bus.channelCount = self.kind.channels() as i32;
        let input = dir as BusDirections == BusDirections_::kInput;
        copy_wstring(if input { "Main In" } else { "Main Out" }, &mut bus.name);
        bus.busType = BusTypes_::kMain as BusType;
        bus.flags = BusInfo_::BusFlags_::kDefaultActive as u32;
        kResultOk
    }

    unsafe fn getRoutingInfo(&self, _in: *mut RoutingInfo, _out: *mut RoutingInfo) -> tresult {
        kNotImplemented
    }

    unsafe fn activateBus(
        &self,
        _media_type: MediaType,
        _dir: BusDirection,
        _index: int32,
        _state: TBool,
    ) -> tresult {
        kResultOk
    }

    unsafe fn setActive(&self, state: TBool) -> tresult {
        let mut a = lock(&self.audio);
        if state == 0 {
            for g in &mut a.gains {
                g.deactivate();
            }
            return kResultOk;
        }
        let setup = *lock(&self.setup);
        let config = ActivateConfig {
            sample_rate: setup.sample_rate,
            max_block: setup.max_block,
            mode: if setup.offline {
                ProcessMode::Offline
            } else {
                ProcessMode::Realtime
            },
            layout: ChannelLayout::MONO,
        };
        let db = self.gain_db();
        for g in &mut a.gains {
            // Values recorded while inactive (state, flush) apply snapped, without a ramp.
            if g.load_state(&Gain::state_with_gain_db(db)).is_err() || g.activate(&config).is_err()
            {
                return kResultFalse;
            }
        }
        let latency = self.latency();
        a.delay = vec![vec![0.0; latency as usize]; self.kind.channels()];
        a.delay_pos = 0;
        a.scratch = vec![0.0; setup.max_block as usize];
        a.steady = 0;
        drop(a);
        self.active_latency.store(latency, Ordering::Release);
        self.announce_latency(latency);
        kResultOk
    }

    unsafe fn setState(&self, state: *mut IBStream) -> tresult {
        // SAFETY: the host's stream, valid for the call.
        let Some((gain, latency)) = unsafe { read_all(state) }
            .as_deref()
            .and_then(parse_component_state)
        else {
            return kResultFalse;
        };
        self.gain_db.store(gain.to_bits(), Ordering::Release);
        if self.kind == Kind::Latency {
            self.latency_param.store(latency, Ordering::Release);
        }
        if C {
            self.ctrl.set_from_component(gain, latency);
        }
        kResultOk
    }

    unsafe fn getState(&self, state: *mut IBStream) -> tresult {
        // SAFETY: the host's stream, valid for the call.
        if unsafe { write_all(state, &self.state_bytes()) } {
            kResultOk
        } else {
            kResultFalse
        }
    }
}

impl<const C: bool> IAudioProcessorTrait for Effect<C> {
    unsafe fn setBusArrangements(
        &self,
        inputs: *mut SpeakerArrangement,
        num_ins: int32,
        outputs: *mut SpeakerArrangement,
        num_outs: int32,
    ) -> tresult {
        if num_ins != 1 || num_outs != 1 || inputs.is_null() || outputs.is_null() {
            return kResultFalse;
        }
        let want = self.kind.arrangement();
        // SAFETY: one arrangement each, as checked.
        if unsafe { *inputs == want && *outputs == want } {
            kResultTrue
        } else {
            kResultFalse
        }
    }

    unsafe fn getBusArrangement(
        &self,
        _dir: BusDirection,
        index: int32,
        arr: *mut SpeakerArrangement,
    ) -> tresult {
        if index != 0 || arr.is_null() {
            return kInvalidArgument;
        }
        // SAFETY: the host's writable arrangement.
        unsafe { *arr = self.kind.arrangement() };
        kResultOk
    }

    unsafe fn canProcessSampleSize(&self, size: int32) -> tresult {
        if size == SymbolicSampleSizes_::kSample32 as i32 {
            kResultTrue
        } else {
            kResultFalse
        }
    }

    unsafe fn getLatencySamples(&self) -> uint32 {
        self.active_latency.load(Ordering::Acquire)
    }

    unsafe fn setupProcessing(&self, setup: *mut ProcessSetup) -> tresult {
        if setup.is_null() {
            return kInvalidArgument;
        }
        // SAFETY: the host's setup, valid for the call.
        let s = unsafe { &*setup };
        if s.symbolicSampleSize != SymbolicSampleSizes_::kSample32 as i32
            || s.maxSamplesPerBlock < 1
        {
            return kResultFalse;
        }
        *lock(&self.setup) = Setup {
            sample_rate: s.sampleRate,
            max_block: s.maxSamplesPerBlock as u32,
            offline: s.processMode == ProcessModes_::kOffline as i32,
        };
        kResultOk
    }

    unsafe fn setProcessing(&self, state: TBool) -> tresult {
        if state != 0 {
            // (Re)starting processing resets the audio state (the host's `reset`).
            let mut a = lock(&self.audio);
            for g in &mut a.gains {
                g.reset();
            }
            for d in &mut a.delay {
                d.fill(0.0);
            }
        }
        kResultOk
    }

    unsafe fn process(&self, data: *mut ProcessData) -> tresult {
        if data.is_null() {
            return kInvalidArgument;
        }
        // SAFETY: the host's process data, valid for the call.
        let d = unsafe { &*data };
        let n = d.numSamples.max(0) as usize;
        let mut guard = lock(&self.audio);
        let a = &mut *guard;
        a.events.clear();
        // SAFETY: null or the host's parameter changes, valid for the call.
        if let Some(changes) = unsafe { ComRef::from_raw(d.inputParameterChanges) } {
            // SAFETY: the host's queues and points are valid for the call.
            unsafe {
                for i in 0..changes.getParameterCount() {
                    let Some(q) = ComRef::from_raw(changes.getParameterData(i)) else {
                        continue;
                    };
                    let id = q.getParameterId();
                    for k in 0..q.getPointCount() {
                        let (mut offset, mut value) = (0i32, 0f64);
                        if q.getPoint(k, &mut offset, &mut value) != kResultOk {
                            continue;
                        }
                        match id {
                            PARAM_GAIN => {
                                let db =
                                    Gain::gain_param().clamp_quantize(normalized_to_gain_db(value));
                                self.gain_db.store(db.to_bits(), Ordering::Release);
                                if n > 0 {
                                    let at = (offset.max(0) as usize).min(n - 1) as u32;
                                    let _ = a.events.push(ParamEvent {
                                        offset: at,
                                        id: Gain::GAIN_DB,
                                        value: db,
                                    });
                                }
                            }
                            PARAM_LATENCY if self.kind == Kind::Latency => self
                                .latency_param
                                .store(normalized_to_latency(value), Ordering::Release),
                            _ => {}
                        }
                    }
                }
            }
        }
        if n == 0 {
            return kResultOk;
        }
        let channels = self.kind.channels();
        if d.numInputs < 1 || d.numOutputs < 1 || d.inputs.is_null() || d.outputs.is_null() {
            return kInvalidArgument;
        }
        // SAFETY: at least one bus each, as checked.
        let (inp, out) = unsafe { (&*d.inputs, &*d.outputs) };
        if (inp.numChannels as usize) < channels
            || (out.numChannels as usize) < channels
            || n > a.scratch.len()
        {
            return kInvalidArgument;
        }
        // SAFETY: 32-bit processing (the only size accepted): the union holds the 32-bit
        // channel pointers.
        let (ins, outs) = unsafe { (inp.__field0.channelBuffers32, out.__field0.channelBuffers32) };
        let Audio {
            gains,
            events,
            out: gain_out,
            delay,
            delay_pos,
            scratch,
            steady,
        } = a;
        let start_pos = *delay_pos;
        for (ch, gain) in gains.iter_mut().enumerate() {
            // SAFETY: the host provides `numChannels` channels of `numSamples` samples.
            let (x, y) = unsafe {
                (
                    std::slice::from_raw_parts(*ins.add(ch), n),
                    std::slice::from_raw_parts_mut(*outs.add(ch), n),
                )
            };
            gain_out.clear();
            let mut ctx = vox_module_api::ProcessContext::new(
                n as u32,
                *steady,
                Transport::default(),
                events.as_slice(),
                gain_out,
            );
            {
                let mut o = [&mut scratch[..n]];
                gain.process(&mut ctx, &[x], &mut o);
            }
            let ring = &mut delay[ch];
            if ring.is_empty() {
                y.copy_from_slice(&scratch[..n]);
            } else {
                let mut pos = start_pos;
                for (o, s) in y.iter_mut().zip(&scratch[..n]) {
                    *o = ring[pos];
                    ring[pos] = *s;
                    pos += 1;
                    if pos == ring.len() {
                        pos = 0;
                    }
                }
                *delay_pos = pos;
            }
        }
        *steady += n as u64;
        kResultOk
    }

    unsafe fn getTailSamples(&self) -> uint32 {
        kNoTail
    }
}

impl<const C: bool> IConnectionPointTrait for Effect<C> {
    unsafe fn connect(&self, other: *mut IConnectionPoint) -> tresult {
        // SAFETY: the host's connection point (or null); we keep a reference.
        *lock(&self.peer) = unsafe { ComRef::from_raw(other) }.map(|p| p.to_com_ptr());
        kResultOk
    }

    unsafe fn disconnect(&self, _other: *mut IConnectionPoint) -> tresult {
        *lock(&self.peer) = None;
        kResultOk
    }

    unsafe fn notify(&self, _message: *mut IMessage) -> tresult {
        kResultOk
    }
}

// --- The controller --------------------------------------------------------------------------

/// The edit-controller logic, shared by the separate [`Controller`] and a combined [`Effect`].
struct ControllerCore {
    kind: Kind,
    /// `Gain`, normalized (bits of an `f64`).
    gain_n: AtomicU64,
    /// `Latency` in samples.
    latency: AtomicU32,
    /// The latency the processor reported at its last activation (`u32::MAX`: not yet known).
    active_latency: AtomicU32,
    ui_scale: AtomicU32,
    handler: Mutex<Option<ComPtr<IComponentHandler>>>,
}

impl ControllerCore {
    fn new(kind: Kind) -> Self {
        Self {
            kind,
            gain_n: AtomicU64::new(gain_db_to_normalized(0.0).to_bits()),
            latency: AtomicU32::new(LATENCY_SAMPLES),
            active_latency: AtomicU32::new(u32::MAX),
            ui_scale: AtomicU32::new(UI_SCALE),
            handler: Mutex::new(None),
        }
    }

    fn set_from_component(&self, gain_db: f64, latency: u32) {
        self.gain_n
            .store(gain_db_to_normalized(gain_db).to_bits(), Ordering::Release);
        self.latency.store(latency, Ordering::Release);
    }

    fn has(&self, id: ParamID) -> bool {
        id == PARAM_GAIN || (id == PARAM_LATENCY && self.kind == Kind::Latency)
    }

    fn get(&self, id: ParamID) -> f64 {
        match id {
            PARAM_GAIN => f64::from_bits(self.gain_n.load(Ordering::Acquire)),
            PARAM_LATENCY => latency_to_normalized(self.latency.load(Ordering::Acquire)),
            _ => 0.0,
        }
    }

    fn set(&self, id: ParamID, n: f64) -> tresult {
        match id {
            PARAM_GAIN => {
                self.gain_n
                    .store(n.clamp(0.0, 1.0).to_bits(), Ordering::Release);
                kResultOk
            }
            PARAM_LATENCY if self.kind == Kind::Latency => {
                let samples = normalized_to_latency(n);
                self.latency.store(samples, Ordering::Release);
                if samples != self.active_latency.load(Ordering::Acquire) {
                    let handler = lock(&self.handler).clone();
                    if let Some(h) = handler {
                        // SAFETY: the host's component handler, kept alive by our reference.
                        unsafe { h.restartComponent(RestartFlags_::kLatencyChanged as i32) };
                    }
                }
                kResultOk
            }
            _ => kInvalidArgument,
        }
    }

    fn plain(&self, id: ParamID, n: f64) -> f64 {
        match id {
            PARAM_GAIN => normalized_to_gain_db(n),
            PARAM_LATENCY => f64::from(normalized_to_latency(n)),
            _ => 0.0,
        }
    }

    fn normalized(&self, id: ParamID, plain: f64) -> f64 {
        match id {
            PARAM_GAIN => gain_db_to_normalized(plain),
            PARAM_LATENCY => latency_to_normalized(plain.round().clamp(0.0, 4096.0) as u32),
            _ => 0.0,
        }
    }

    fn text(&self, id: ParamID, n: f64) -> Option<String> {
        match id {
            PARAM_GAIN => Some(format!("{:.2}", normalized_to_gain_db(n))),
            PARAM_LATENCY if self.kind == Kind::Latency => {
                Some(format!("{}", normalized_to_latency(n)))
            }
            _ => None,
        }
    }

    /// Parses a plain number (no units: the host strips them).
    fn parse(&self, id: ParamID, text: &str) -> Option<f64> {
        let v: f64 = text.trim().parse().ok().filter(|v: &f64| v.is_finite())?;
        self.has(id).then(|| self.normalized(id, v))
    }

    fn info(&self, index: i32, info: &mut ParameterInfo) -> tresult {
        match index {
            0 => {
                info.id = PARAM_GAIN;
                copy_wstring("Gain", &mut info.title);
                copy_wstring("Gain", &mut info.shortTitle);
                copy_wstring("dB", &mut info.units);
                info.stepCount = 0;
                info.defaultNormalizedValue = gain_db_to_normalized(0.0);
                info.unitId = kRootUnitId;
                info.flags = ParameterInfo_::ParameterFlags_::kCanAutomate as i32;
                kResultOk
            }
            1 if self.kind == Kind::Latency => {
                info.id = PARAM_LATENCY;
                copy_wstring("Latency", &mut info.title);
                copy_wstring("Latency", &mut info.shortTitle);
                copy_wstring("smp", &mut info.units);
                info.stepCount = MAX_LATENCY as i32;
                info.defaultNormalizedValue = latency_to_normalized(LATENCY_SAMPLES);
                info.unitId = ADVANCED_UNIT;
                info.flags = 0;
                kResultOk
            }
            _ => kInvalidArgument,
        }
    }

    fn state_bytes(&self) -> Vec<u8> {
        let mut b = Vec::with_capacity(12);
        b.extend_from_slice(&CONTROLLER_STATE_MAGIC);
        b.extend_from_slice(&1u32.to_le_bytes());
        b.extend_from_slice(&self.ui_scale.load(Ordering::Acquire).to_le_bytes());
        b
    }

    fn load_state_bytes(&self, b: &[u8]) -> bool {
        if b.len() != 12 || b[..4] != CONTROLLER_STATE_MAGIC || b[4..8] != 1u32.to_le_bytes() {
            return false;
        }
        let scale = u32::from_le_bytes([b[8], b[9], b[10], b[11]]);
        self.ui_scale.store(scale, Ordering::Release);
        true
    }
}

/// A separate edit controller (`ID_GAIN`, `ID_LATENCY`).
struct Controller {
    core: ControllerCore,
    peer: Mutex<Option<ComPtr<IConnectionPoint>>>,
}

impl Class for Controller {
    type Interfaces = (IEditController, IConnectionPoint, IUnitInfo);
}

impl IPluginBaseTrait for Controller {
    unsafe fn initialize(&self, _context: *mut FUnknown) -> tresult {
        kResultOk
    }

    unsafe fn terminate(&self) -> tresult {
        *lock(&self.peer) = None;
        *lock(&self.core.handler) = None;
        kResultOk
    }
}

impl IConnectionPointTrait for Controller {
    unsafe fn connect(&self, other: *mut IConnectionPoint) -> tresult {
        // SAFETY: the host's connection point (or null); we keep a reference.
        *lock(&self.peer) = unsafe { ComRef::from_raw(other) }.map(|p| p.to_com_ptr());
        kResultOk
    }

    unsafe fn disconnect(&self, _other: *mut IConnectionPoint) -> tresult {
        *lock(&self.peer) = None;
        kResultOk
    }

    unsafe fn notify(&self, message: *mut IMessage) -> tresult {
        // SAFETY: the sender's message (or null), valid for the call.
        let Some(msg) = (unsafe { ComRef::from_raw(message) }) else {
            return kInvalidArgument;
        };
        // SAFETY: a live message; its id is NUL-terminated or null.
        let id = unsafe { msg.getMessageID() };
        // SAFETY: as above.
        if id.is_null() || unsafe { CStr::from_ptr(id) } != ACTIVE_LATENCY_MESSAGE {
            return kResultFalse;
        }
        // SAFETY: a live message's attribute list (or null).
        let Some(attrs) = (unsafe { ComRef::from_raw(msg.getAttributes()) }) else {
            return kResultFalse;
        };
        let mut samples = 0i64;
        // SAFETY: a live attribute list; NUL-terminated id; writable `samples`.
        if unsafe { attrs.getInt(SAMPLES_ATTR.as_ptr(), &mut samples) } != kResultOk {
            return kResultFalse;
        }
        let samples = u32::try_from(samples).unwrap_or(u32::MAX);
        self.core.active_latency.store(samples, Ordering::Release);
        kResultOk
    }
}

impl IUnitInfoTrait for Controller {
    unsafe fn getUnitCount(&self) -> int32 {
        if self.core.kind == Kind::Latency {
            2
        } else {
            1
        }
    }

    unsafe fn getUnitInfo(&self, index: int32, info: *mut UnitInfo) -> tresult {
        if info.is_null() || index < 0 || index >= self.core.kind.param_count() {
            return kInvalidArgument;
        }
        // SAFETY: the host's writable unit info.
        let info = unsafe { &mut *info };
        if index == 0 {
            info.id = kRootUnitId;
            info.parentUnitId = kNoParentUnitId;
            copy_wstring("Root", &mut info.name);
        } else {
            info.id = ADVANCED_UNIT;
            info.parentUnitId = kRootUnitId;
            copy_wstring("Advanced", &mut info.name);
        }
        info.programListId = kNoProgramListId;
        kResultOk
    }

    unsafe fn getProgramListCount(&self) -> int32 {
        0
    }

    unsafe fn getProgramListInfo(&self, _index: int32, _info: *mut ProgramListInfo) -> tresult {
        kInvalidArgument
    }

    unsafe fn getProgramName(
        &self,
        _list: ProgramListID,
        _index: int32,
        _name: *mut String128,
    ) -> tresult {
        kInvalidArgument
    }

    unsafe fn getProgramInfo(
        &self,
        _list: ProgramListID,
        _index: int32,
        _attribute: *const c_char,
        _value: *mut String128,
    ) -> tresult {
        kInvalidArgument
    }

    unsafe fn hasProgramPitchNames(&self, _list: ProgramListID, _index: int32) -> tresult {
        kResultFalse
    }

    unsafe fn getProgramPitchName(
        &self,
        _list: ProgramListID,
        _index: int32,
        _pitch: int16,
        _name: *mut String128,
    ) -> tresult {
        kResultFalse
    }

    unsafe fn getSelectedUnit(&self) -> UnitID {
        kRootUnitId
    }

    unsafe fn selectUnit(&self, _unit: UnitID) -> tresult {
        kResultOk
    }

    unsafe fn getUnitByBus(
        &self,
        _media_type: MediaType,
        _dir: BusDirection,
        _bus: int32,
        _channel: int32,
        _unit: *mut UnitID,
    ) -> tresult {
        kResultFalse
    }

    unsafe fn setUnitProgramData(
        &self,
        _list_or_unit: int32,
        _index: int32,
        _data: *mut IBStream,
    ) -> tresult {
        kNotImplemented
    }
}

/// `IEditControllerTrait` for a type whose `$core` field is the [`ControllerCore`].
macro_rules! edit_controller {
    ($ty:ty, $core:ident) => {
        impl IEditControllerTrait for $ty {
            unsafe fn setComponentState(&self, state: *mut IBStream) -> tresult {
                // SAFETY: the host's stream, valid for the call.
                match unsafe { read_all(state) }
                    .as_deref()
                    .and_then(parse_component_state)
                {
                    Some((gain, latency)) => {
                        self.$core.set_from_component(gain, latency);
                        kResultOk
                    }
                    None => kResultFalse,
                }
            }

            unsafe fn setState(&self, state: *mut IBStream) -> tresult {
                // SAFETY: the host's stream, valid for the call.
                let ok =
                    unsafe { read_all(state) }.is_some_and(|b| self.$core.load_state_bytes(&b));
                if ok { kResultOk } else { kResultFalse }
            }

            unsafe fn getState(&self, state: *mut IBStream) -> tresult {
                // SAFETY: the host's stream, valid for the call.
                if unsafe { write_all(state, &self.$core.state_bytes()) } {
                    kResultOk
                } else {
                    kResultFalse
                }
            }

            unsafe fn getParameterCount(&self) -> int32 {
                self.$core.kind.param_count()
            }

            unsafe fn getParameterInfo(&self, index: int32, info: *mut ParameterInfo) -> tresult {
                if info.is_null() {
                    return kInvalidArgument;
                }
                // SAFETY: the host's writable parameter info.
                self.$core.info(index, unsafe { &mut *info })
            }

            unsafe fn getParamStringByValue(
                &self,
                id: ParamID,
                value: ParamValue,
                string: *mut String128,
            ) -> tresult {
                match self.$core.text(id, value) {
                    Some(t) if !string.is_null() => {
                        // SAFETY: the host's writable String128.
                        copy_wstring(&t, unsafe { &mut *string });
                        kResultOk
                    }
                    _ => kInvalidArgument,
                }
            }

            unsafe fn getParamValueByString(
                &self,
                id: ParamID,
                string: *mut TChar,
                value: *mut ParamValue,
            ) -> tresult {
                if string.is_null() || value.is_null() {
                    return kInvalidArgument;
                }
                // SAFETY: a NUL-terminated UTF-16 string from the host.
                let text = unsafe { read_wstring(string) };
                match self.$core.parse(id, &text) {
                    Some(n) => {
                        // SAFETY: writable per the contract.
                        unsafe { *value = n };
                        kResultOk
                    }
                    None => kResultFalse,
                }
            }

            unsafe fn normalizedParamToPlain(&self, id: ParamID, value: ParamValue) -> ParamValue {
                self.$core.plain(id, value)
            }

            unsafe fn plainParamToNormalized(&self, id: ParamID, plain: ParamValue) -> ParamValue {
                self.$core.normalized(id, plain)
            }

            unsafe fn getParamNormalized(&self, id: ParamID) -> ParamValue {
                self.$core.get(id)
            }

            unsafe fn setParamNormalized(&self, id: ParamID, value: ParamValue) -> tresult {
                self.$core.set(id, value)
            }

            unsafe fn setComponentHandler(&self, handler: *mut IComponentHandler) -> tresult {
                // SAFETY: the host's handler (or null); we keep a reference.
                *lock(&self.$core.handler) =
                    unsafe { ComRef::from_raw(handler) }.map(|h| h.to_com_ptr());
                kResultOk
            }

            unsafe fn createView(&self, _name: FIDString) -> *mut IPlugView {
                std::ptr::null_mut()
            }
        }
    };
}

edit_controller!(Controller, core);
edit_controller!(Effect<true>, ctrl);

// --- Factory and entry points ----------------------------------------------------------------

struct ClassDef {
    cid: TUID,
    category: &'static str,
    name: &'static str,
    sub_categories: &'static str,
}

const AUDIO: &str = "Audio Module Class";
const CONTROLLER: &str = "Component Controller Class";

static CLASSES: [ClassDef; 5] = [
    ClassDef {
        cid: CID_GAIN,
        category: AUDIO,
        name: "Test Gain",
        sub_categories: "Fx|Tools|Mono",
    },
    ClassDef {
        cid: CID_GAIN_CTRL,
        category: CONTROLLER,
        name: "Test Gain",
        sub_categories: "",
    },
    ClassDef {
        cid: CID_GAIN_STEREO,
        category: AUDIO,
        name: "Test Gain (stereo)",
        sub_categories: "Fx|Tools|Stereo",
    },
    ClassDef {
        cid: CID_LATENCY,
        category: AUDIO,
        name: "Test Latency",
        sub_categories: "Fx|Tools|Mono",
    },
    ClassDef {
        cid: CID_LATENCY_CTRL,
        category: CONTROLLER,
        name: "Test Latency",
        sub_categories: "",
    },
];

struct Factory;

impl Class for Factory {
    type Interfaces = (IPluginFactory2,);
}

impl IPluginFactoryTrait for Factory {
    unsafe fn getFactoryInfo(&self, info: *mut PFactoryInfo) -> tresult {
        if info.is_null() {
            return kInvalidArgument;
        }
        // SAFETY: the host's writable factory info.
        let info = unsafe { &mut *info };
        copy_cstring("PowerVoice", &mut info.vendor);
        copy_cstring("https://powervoice.app", &mut info.url);
        copy_cstring("", &mut info.email);
        info.flags = PFactoryInfo_::FactoryFlags_::kUnicode as int32;
        kResultOk
    }

    unsafe fn countClasses(&self) -> int32 {
        CLASSES.len() as int32
    }

    unsafe fn getClassInfo(&self, index: int32, info: *mut PClassInfo) -> tresult {
        let Some(c) = usize::try_from(index).ok().and_then(|i| CLASSES.get(i)) else {
            return kInvalidArgument;
        };
        if info.is_null() {
            return kInvalidArgument;
        }
        // SAFETY: the host's writable class info.
        let info = unsafe { &mut *info };
        info.cid = c.cid;
        info.cardinality = PClassInfo_::ClassCardinality_::kManyInstances as int32;
        copy_cstring(c.category, &mut info.category);
        copy_cstring(c.name, &mut info.name);
        kResultOk
    }

    unsafe fn createInstance(
        &self,
        cid: FIDString,
        iid: FIDString,
        obj: *mut *mut c_void,
    ) -> tresult {
        if cid.is_null() || iid.is_null() || obj.is_null() {
            return kInvalidArgument;
        }
        // SAFETY: a class id is 16 bytes.
        let cid = unsafe { *(cid as *const TUID) };
        let unknown = if cid == CID_GAIN {
            ComWrapper::new(Effect::<false>::new(Kind::Mono)).to_com_ptr::<FUnknown>()
        } else if cid == CID_GAIN_STEREO {
            ComWrapper::new(Effect::<true>::new(Kind::Stereo)).to_com_ptr::<FUnknown>()
        } else if cid == CID_LATENCY {
            ComWrapper::new(Effect::<false>::new(Kind::Latency)).to_com_ptr::<FUnknown>()
        } else if cid == CID_GAIN_CTRL || cid == CID_LATENCY_CTRL {
            let kind = if cid == CID_GAIN_CTRL {
                Kind::Mono
            } else {
                Kind::Latency
            };
            ComWrapper::new(Controller {
                core: ControllerCore::new(kind),
                peer: Mutex::new(None),
            })
            .to_com_ptr::<FUnknown>()
        } else {
            None
        };
        let Some(unknown) = unknown else {
            // SAFETY: writable per the contract.
            unsafe { *obj = std::ptr::null_mut() };
            return kInvalidArgument;
        };
        let ptr = unknown.as_ptr();
        // SAFETY: our own object; `queryInterface` adds the reference the caller owns.
        unsafe { ((*(*ptr).vtbl).queryInterface)(ptr, iid as *const TUID, obj) }
    }
}

impl IPluginFactory2Trait for Factory {
    unsafe fn getClassInfo2(&self, index: int32, info: *mut PClassInfo2) -> tresult {
        let Some(c) = usize::try_from(index).ok().and_then(|i| CLASSES.get(i)) else {
            return kInvalidArgument;
        };
        if info.is_null() {
            return kInvalidArgument;
        }
        // SAFETY: the host's writable class info.
        let info = unsafe { &mut *info };
        info.cid = c.cid;
        info.cardinality = PClassInfo_::ClassCardinality_::kManyInstances as int32;
        copy_cstring(c.category, &mut info.category);
        copy_cstring(c.name, &mut info.name);
        info.classFlags = 0;
        copy_cstring(c.sub_categories, &mut info.subCategories);
        copy_cstring("PowerVoice", &mut info.vendor);
        copy_cstring("1.2.3", &mut info.version);
        copy_cstring("VST 3.8.0", &mut info.sdkVersion);
        kResultOk
    }
}

/// The deliberate "crashing bundle during scan" abort — without a core dump (an intentional test
/// crash must not reach systemd-coredump's desktop notifications). Dies by `SIGABRT`.
#[cfg(unix)]
fn crash_without_core_dump() -> ! {
    // SAFETY: setrlimit with a valid struct; it only lowers this process's own core limit.
    unsafe {
        libc::setrlimit(
            libc::RLIMIT_CORE,
            &libc::rlimit {
                rlim_cur: 0,
                rlim_max: 0,
            },
        );
    }
    #[cfg(target_os = "linux")]
    // SAFETY: PR_SET_DUMPABLE 0 only marks this process non-dumpable.
    unsafe {
        libc::prctl(libc::PR_SET_DUMPABLE, 0, 0, 0, 0);
    }
    std::process::abort()
}

/// This library's own file name (`dladdr` on one of its functions).
#[cfg(unix)]
fn own_file_name() -> Option<String> {
    // SAFETY: zeroed POD filled by dladdr.
    let mut info: libc::Dl_info = unsafe { std::mem::zeroed() };
    // SAFETY: the address of a function of this library; `info` is writable.
    let found = unsafe { libc::dladdr(GetPluginFactory as *const c_void, &mut info) };
    if found == 0 || info.dli_fname.is_null() {
        return None;
    }
    // SAFETY: dladdr's NUL-terminated file name.
    let path = unsafe { CStr::from_ptr(info.dli_fname) }.to_string_lossy();
    path.rsplit('/').next().map(str::to_owned)
}

/// The scan-time fault doubles (see the crate docs).
#[cfg(unix)]
fn scan_faults() {
    let Some(name) = own_file_name() else {
        return;
    };
    if name.contains(CRASH_ON_SCAN_MARKER) {
        crash_without_core_dump();
    }
    if name.contains(HANG_ON_SCAN_MARKER) {
        loop {
            std::thread::sleep(std::time::Duration::from_secs(3600));
        }
    }
}

/// The VST3 module entry (Linux).
#[cfg(target_os = "linux")]
#[unsafe(no_mangle)]
pub extern "system" fn ModuleEntry(_library_handle: *mut c_void) -> bool {
    scan_faults();
    true
}

/// The VST3 module exit (Linux).
#[cfg(target_os = "linux")]
#[unsafe(no_mangle)]
pub extern "system" fn ModuleExit() -> bool {
    true
}

/// The VST3 bundle entry (macOS).
#[cfg(target_os = "macos")]
#[unsafe(no_mangle)]
pub extern "system" fn bundleEntry(_bundle: *mut c_void) -> bool {
    scan_faults();
    true
}

/// The VST3 bundle exit (macOS).
#[cfg(target_os = "macos")]
#[unsafe(no_mangle)]
pub extern "system" fn bundleExit() -> bool {
    true
}

/// The VST3 DLL entry (Windows).
#[cfg(windows)]
#[unsafe(no_mangle)]
pub extern "system" fn InitDll() -> bool {
    true
}

/// The VST3 DLL exit (Windows).
#[cfg(windows)]
#[unsafe(no_mangle)]
pub extern "system" fn ExitDll() -> bool {
    true
}

/// The VST3 factory entry point: a new reference the caller owns.
#[unsafe(no_mangle)]
pub extern "system" fn GetPluginFactory() -> *mut IPluginFactory {
    ComWrapper::new(Factory)
        .to_com_ptr::<IPluginFactory>()
        .map_or(std::ptr::null_mut(), ComPtr::into_raw)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The FUID string of a class id: its four 32-bit words, `%08X` each (all platforms).
    fn fuid_string(words: [u32; 4]) -> String {
        words.iter().map(|w| format!("{w:08X}")).collect()
    }

    #[test]
    fn class_ids_match_their_fuid_strings() {
        let t = |a, b, c, d| fuid_string([a, b, c, d]);
        assert_eq!(t(0x5056_5633, 0x5445_5354, 0x4741_494E, 1), ID_GAIN);
        assert_eq!(t(0x5056_5633, 0x5445_5354, 0x4741_494E, 2), ID_GAIN_STEREO);
        assert_eq!(t(0x5056_5633, 0x5445_5354, 0x4C41_5459, 1), ID_LATENCY);
    }

    #[test]
    fn value_mappings_round_trip_exactly() {
        for db in [
            -60.0, -20.0, -12.0, -9.5, -7.5, -6.0, -4.5, -3.0, 0.0, 6.0, 24.0,
        ] {
            let got = normalized_to_gain_db(gain_db_to_normalized(db));
            assert_eq!(got.to_bits(), f64::to_bits(db), "{db}");
        }
        for s in [0, 1, 64, 300, 4095, 4096] {
            assert_eq!(normalized_to_latency(latency_to_normalized(s)), s);
        }
    }
}

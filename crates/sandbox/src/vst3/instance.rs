//! One VST3 audio effect as a sandbox [`PluginInstance`].

use std::path::Path;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use vox_module_api::{ActivateConfig, ParamGroup, ParamId, ParamInfo, ProcessMode};
use vox_sandbox_ipc::protocol::{ParamValue, PluginInfo};
use vox_sandbox_ipc::vst3::AUDIO_MODULE_CLASS;
use vox_sandbox_ipc::{Chunk, EventKind, WireEvent};
use vst3::Steinberg::Vst::BusDirections_::{kInput, kOutput};
use vst3::Steinberg::Vst::BusTypes_::kMain;
use vst3::Steinberg::Vst::MediaTypes_::{kAudio, kEvent};
use vst3::Steinberg::Vst::ProcessContext_::StatesAndFlags_::{kTempoValid, kTimeSigValid};
use vst3::Steinberg::Vst::ProcessModes_::{kOffline, kRealtime};
use vst3::Steinberg::Vst::SymbolicSampleSizes_::kSample32;
use vst3::Steinberg::Vst::{
    AudioBusBuffers, AudioBusBuffers__type0, BusDirection, BusInfo, IAudioProcessor,
    IAudioProcessorTrait, IComponent, IComponentHandler, IComponentTrait, IConnectionPoint,
    IConnectionPointTrait, IEditController, IEditControllerTrait, IParameterChanges, IUnitInfo,
    IUnitInfoTrait, MediaType, ParameterInfo, ProcessContext, ProcessData, ProcessSetup,
    SpeakerArr, SpeakerArrangement, String128, TChar, UnitInfo, kInfiniteTail,
};
use vst3::Steinberg::{FUnknown, IPluginBaseTrait, TUID, kResultOk, kResultTrue};
use vst3::{ComPtr, ComWrapper};

use super::host::{
    ComponentHandler, HostApplication, ParamChanges, PluginEdit, Shared, changes_ptr, read_wstring,
};
use super::module::Vst3Module;
use super::params::{self, Domain, RawParam};
use super::{stream, uid};
use crate::backend::{ActiveInfo, PluginInstance};

/// Parameters one block forwards at most (a queue each; more are dropped).
const MAX_BLOCK_PARAMS: usize = 64;
/// Points per parameter and block (the segment's event ring is smaller anyway).
const MAX_BLOCK_POINTS: usize = 512;
/// Output parameters (and their points) one block reports at most.
const MAX_OUT_PARAMS: usize = 64;
const MAX_OUT_POINTS: usize = 64;
/// State framing: magic, version, then the component state and the controller state, each as a
/// little-endian `u64` length and the bytes.
const STATE_MAGIC: [u8; 4] = *b"PVV3";
const STATE_VERSION: u32 = 1;

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Frames the component and controller states (ADR-008 Amendment 6 §4).
pub(crate) fn frame_state(component: &[u8], controller: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(24 + component.len() + controller.len());
    out.extend_from_slice(&STATE_MAGIC);
    out.extend_from_slice(&STATE_VERSION.to_le_bytes());
    for part in [component, controller] {
        out.extend_from_slice(&(part.len() as u64).to_le_bytes());
        out.extend_from_slice(part);
    }
    out
}

/// The component and controller states of a [`frame_state`] blob.
pub(crate) fn unframe_state(data: &[u8]) -> Option<(&[u8], &[u8])> {
    let rest = data.strip_prefix(&STATE_MAGIC)?;
    let (version, mut rest) = rest.split_first_chunk::<4>()?;
    if u32::from_le_bytes(*version) != STATE_VERSION {
        return None;
    }
    let mut part = || -> Option<&[u8]> {
        let (len, tail) = rest.split_first_chunk::<8>()?;
        let len = usize::try_from(u64::from_le_bytes(*len)).ok()?;
        (tail.len() >= len).then(|| {
            let (p, t) = tail.split_at(len);
            rest = t;
            p
        })
    };
    let component = part()?;
    let controller = part()?;
    rest.is_empty().then_some((component, controller))
}

/// The plugin's audio buses.
#[derive(Clone, Debug, Default)]
struct Buses {
    /// Channel counts per bus.
    inputs: Vec<u32>,
    outputs: Vec<u32>,
    main_in: usize,
    main_out: usize,
}

/// The audio thread's working set, built at activation (heap buffers: the pointers stay valid).
struct AudioState {
    max_block: usize,
    in_bufs: Vec<Vec<Vec<f32>>>,
    out_bufs: Vec<Vec<Vec<f32>>>,
    /// Kept alive for `in_audio`/`out_audio`'s channel pointers.
    _in_ptrs: Vec<Vec<*mut f32>>,
    _out_ptrs: Vec<Vec<*mut f32>>,
    in_audio: Vec<AudioBusBuffers>,
    out_audio: Vec<AudioBusBuffers>,
    main_in: usize,
    main_out: usize,
    in_changes: ComWrapper<ParamChanges>,
    out_changes: ComWrapper<ParamChanges>,
    in_changes_ptr: *mut IParameterChanges,
    out_changes_ptr: *mut IParameterChanges,
    context: Box<ProcessContext>,
    process_mode: i32,
    steady: i64,
    processing: bool,
}

// SAFETY: the raw pointers point into this struct's own heap buffers and COM objects; the state
// moves between the main thread (activation) and the audio thread (processing) behind a mutex.
unsafe impl Send for AudioState {}

impl AudioState {
    fn process_data(&mut self, frames: usize) -> ProcessData {
        ProcessData {
            processMode: self.process_mode,
            symbolicSampleSize: kSample32 as i32,
            numSamples: frames as i32,
            numInputs: self.in_audio.len() as i32,
            numOutputs: self.out_audio.len() as i32,
            inputs: self.in_audio.as_mut_ptr(),
            outputs: self.out_audio.as_mut_ptr(),
            inputParameterChanges: self.in_changes_ptr,
            outputParameterChanges: self.out_changes_ptr,
            inputEvents: std::ptr::null_mut(),
            outputEvents: std::ptr::null_mut(),
            processContext: &raw mut *self.context,
        }
    }
}

/// A loaded VST3 audio effect (ADR-008 Amendment 6).
pub struct Vst3Instance {
    name: String,
    vendor: String,
    version: String,
    params: Vec<ParamInfo>,
    groups: Vec<ParamGroup>,
    /// `(id, units)`, sorted by id: appended to the plugin's value text.
    units: Vec<(u32, String)>,
    component: ComPtr<IComponent>,
    processor: ComPtr<IAudioProcessor>,
    controller: Option<ComPtr<IEditController>>,
    /// The controller is its own object (initialised, connected and terminated separately).
    separate: bool,
    connection: Option<(ComPtr<IConnectionPoint>, ComPtr<IConnectionPoint>)>,
    shared: Arc<Shared>,
    _host: ComWrapper<HostApplication>,
    handler: ComWrapper<ComponentHandler>,
    buses: Buses,
    /// Values set while inactive (`(id, normalized)`), flushed at the next activation.
    pending: Mutex<Vec<(u32, f64)>>,
    active: Mutex<bool>,
    audio: Mutex<Option<Box<AudioState>>>,
    /// Dropped last: the module exit and unload run after every object was released.
    _module: Arc<Vst3Module>,
}

// Threading: the VST3 contract is kept by the sandbox — `[UI-thread]` calls come from the
// control loop's thread only, `setProcessing`/`process` from the audio thread only (the audio
// state is behind a mutex); every COM pointer stays valid until `Drop`.

/// The channel count of a speaker arrangement.
fn channels_of(arr: SpeakerArrangement) -> u32 {
    arr.count_ones()
}

impl Vst3Instance {
    /// Loads processor class `cid` (FUID string) from `path` (main thread: the caller's thread
    /// becomes the plugin's UI thread).
    pub fn load(path: &Path, cid: &str) -> Result<Self, String> {
        Self::load_in(Arc::new(Vst3Module::open(path)?), cid)
    }

    /// [`Self::load`] from an already loaded module (the scan loads each file once).
    pub(crate) fn load_in(module: Arc<Vst3Module>, cid: &str) -> Result<Self, String> {
        let tuid: TUID = uid::from_hex(cid).ok_or_else(|| format!("bad VST3 class id `{cid}`"))?;
        let class = module
            .classes()
            .into_iter()
            .find(|c| c.cid == tuid && c.category == AUDIO_MODULE_CLASS)
            .ok_or_else(|| format!("the plugin has no audio processor class {cid}"))?;
        let name = if class.name.is_empty() {
            cid.to_owned()
        } else {
            class.name.clone()
        };
        let host = ComWrapper::new(HostApplication);
        let context = host
            .as_com_ref::<FUnknown>()
            .map_or(std::ptr::null_mut(), |r| r.as_ptr());
        let component: ComPtr<IComponent> = module
            .create(&tuid)
            .ok_or_else(|| format!("{name} couldn't be created"))?;
        // SAFETY: a fresh component, main thread; `initialize` once, with the host context.
        if unsafe { component.initialize(context) } != kResultOk {
            return Err(format!("{name} failed to initialise"));
        }
        let Some(processor) = component.cast::<IAudioProcessor>() else {
            // SAFETY: initialised above; terminate once.
            unsafe { component.terminate() };
            return Err(format!("{name} is not an audio processor"));
        };
        let (controller, separate) = match component.cast::<IEditController>() {
            Some(c) => (Some(c), false),
            None => (
                Self::separate_controller(&module, &component, context),
                true,
            ),
        };
        let shared = Arc::new(Shared::new(Vec::new()));
        let mut me = Self {
            name,
            vendor: class.vendor,
            version: class.version,
            params: Vec::new(),
            groups: Vec::new(),
            units: Vec::new(),
            component,
            processor,
            separate: separate && controller.is_some(),
            controller,
            connection: None,
            handler: ComWrapper::new(ComponentHandler {
                shared: shared.clone(),
            }),
            shared,
            _host: host,
            buses: Buses::default(),
            pending: Mutex::new(Vec::new()),
            active: Mutex::new(false),
            audio: Mutex::new(None),
            _module: module,
        };
        // From here on, `Drop` disconnects and terminates on every error path.
        me.join_controller();
        me.read_params();
        me.buses = me.read_buses()?;
        Ok(me)
    }

    /// Creates and initialises the component's separate controller, if it names one.
    fn separate_controller(
        module: &Vst3Module,
        component: &ComPtr<IComponent>,
        context: *mut FUnknown,
    ) -> Option<ComPtr<IEditController>> {
        let mut cid: TUID = [0; 16];
        // SAFETY: an initialised component; `cid` is writable.
        if unsafe { component.getControllerClassId(&mut cid) } != kResultOk || cid == [0; 16] {
            return None;
        }
        let controller: ComPtr<IEditController> = module.create(&cid)?;
        // SAFETY: a fresh controller, main thread; `initialize` once, with the host context.
        (unsafe { controller.initialize(context) } == kResultOk).then_some(controller)
    }

    /// Connects a separate controller to the component and syncs it with the component's state.
    fn join_controller(&mut self) {
        let Some(controller) = self.controller.clone() else {
            return;
        };
        if self.separate {
            if let (Some(a), Some(b)) = (
                self.component.cast::<IConnectionPoint>(),
                controller.cast::<IConnectionPoint>(),
            ) {
                // SAFETY: both initialised; main thread; each side keeps its own reference.
                unsafe {
                    a.connect(b.as_ptr());
                    b.connect(a.as_ptr());
                }
                self.connection = Some((a, b));
            }
            // SAFETY: main thread; the stream lives for the call.
            if let Ok(state) = stream::save(|s| unsafe { self.component.getState(s) }) {
                // SAFETY: as above.
                stream::load(&state, |s| unsafe { controller.setComponentState(s) });
            }
        }
    }

    /// The controller's parameters (schema, domains, units) and the handler that knows them.
    fn read_params(&mut self) {
        let Some(c) = self.controller.clone() else {
            return;
        };
        let mut raw = Vec::new();
        let mut units = Vec::new();
        // SAFETY: an initialised controller, main thread.
        for i in 0..unsafe { c.getParameterCount() } {
            // SAFETY: zeroed POD the controller fills.
            let mut info: ParameterInfo = unsafe { std::mem::zeroed() };
            // SAFETY: `i < getParameterCount`; `info` is writable.
            if unsafe { c.getParameterInfo(i, &mut info) } != kResultOk {
                continue;
            }
            units.push((info.id, read_wstring(&info.units).trim().to_owned()));
            raw.push(RawParam {
                id: info.id,
                title: read_wstring(&info.title),
                step_count: info.stepCount,
                default_normalized: info.defaultNormalizedValue,
                unit_id: info.unitId,
                flags: info.flags,
            });
        }
        let unit_names = Self::unit_names(&c);
        let (params, groups, domains) =
            params::map(&raw, &unit_names, |id, n| Self::value_string(&c, id, n));
        units.sort_unstable_by_key(|u| u.0);
        units.dedup_by_key(|u| u.0);
        self.params = params;
        self.groups = groups;
        self.units = units;
        self.shared = Arc::new(Shared::new(domains));
        self.handler = ComWrapper::new(ComponentHandler {
            shared: self.shared.clone(),
        });
        let handler = self
            .handler
            .as_com_ref::<IComponentHandler>()
            .map_or(std::ptr::null_mut(), |r| r.as_ptr());
        // SAFETY: main thread; the handler outlives the controller's use of it (`Drop`
        // terminates the controller before the handler is released).
        unsafe { c.setComponentHandler(handler) };
    }

    /// `IUnitInfo`'s `(unit id, name)` list (empty without it).
    fn unit_names(c: &ComPtr<IEditController>) -> Vec<(i32, String)> {
        let Some(units) = c.cast::<IUnitInfo>() else {
            return Vec::new();
        };
        let mut out = Vec::new();
        // SAFETY: the controller's unit info, main thread.
        for i in 0..unsafe { units.getUnitCount() } {
            // SAFETY: zeroed POD the controller fills.
            let mut u: UnitInfo = unsafe { std::mem::zeroed() };
            // SAFETY: `i < getUnitCount`; `u` is writable.
            if unsafe { units.getUnitInfo(i, &mut u) } == kResultOk {
                out.push((u.id, read_wstring(&u.name)));
            }
        }
        out
    }

    /// The controller's text for normalized value `n` (without units).
    fn value_string(c: &ComPtr<IEditController>, id: u32, n: f64) -> Option<String> {
        let mut s: String128 = [0; 128];
        // SAFETY: main thread; `s` is a writable String128.
        if unsafe { c.getParamStringByValue(id, n, &mut s) } != kResultOk {
            return None;
        }
        let text = read_wstring(&s).trim().to_owned();
        (!text.is_empty()).then_some(text)
    }

    fn bus_channels(&self, dir: BusDirection) -> (Vec<u32>, usize) {
        let media = kAudio as MediaType;
        // SAFETY: an initialised component, main thread.
        let count = unsafe { self.component.getBusCount(media, dir) }.max(0);
        let mut channels = Vec::with_capacity(count as usize);
        let mut main = None;
        for i in 0..count {
            // SAFETY: zeroed POD the component fills.
            let mut info: BusInfo = unsafe { std::mem::zeroed() };
            // SAFETY: `i < getBusCount`; `info` is writable.
            if unsafe { self.component.getBusInfo(media, dir, i, &mut info) } != kResultOk {
                channels.push(0);
                continue;
            }
            if main.is_none() && info.busType == kMain as i32 && info.channelCount > 0 {
                main = Some(channels.len());
            }
            channels.push(info.channelCount.max(0) as u32);
        }
        let main = main
            .or_else(|| channels.iter().position(|&c| c > 0))
            .unwrap_or(usize::MAX);
        (channels, main)
    }

    fn read_buses(&self) -> Result<Buses, String> {
        let (inputs, main_in) = self.bus_channels(kInput as BusDirection);
        let (outputs, main_out) = self.bus_channels(kOutput as BusDirection);
        if main_in == usize::MAX || main_out == usize::MAX {
            return Err(format!(
                "{} has no audio {} (not an audio effect)",
                self.name,
                if main_in == usize::MAX {
                    "input"
                } else {
                    "output"
                }
            ));
        }
        Ok(Buses {
            inputs,
            outputs,
            main_in,
            main_out,
        })
    }

    /// Parameter count (T-804 richer scan data, ADR-008 §6).
    pub(crate) fn param_count(&self) -> u32 {
        self.params.len() as u32
    }

    /// `(main input channels, main output channels)` before any arrangement is negotiated.
    pub(crate) fn main_ports(&self) -> (u32, u32) {
        (
            self.buses.inputs[self.buses.main_in],
            self.buses.outputs[self.buses.main_out],
        )
    }

    fn domain(&self, id: u32) -> Option<Domain> {
        self.shared.slot(id).map(|s| s.domain)
    }

    fn units_of(&self, id: u32) -> &str {
        self.units
            .binary_search_by_key(&id, |u| u.0)
            .map_or("", |i| self.units[i].1.as_str())
    }

    fn current_values(&self) -> Vec<ParamValue> {
        self.params
            .iter()
            .map(|p| {
                let mut v = p.default;
                if let (Some(c), Some(d)) = (&self.controller, self.domain(p.id.0)) {
                    // SAFETY: main thread.
                    let n = unsafe { c.getParamNormalized(p.id.0) };
                    if n.is_finite() {
                        v = d.value_of(n);
                    }
                }
                ParamValue {
                    id: p.id,
                    value: v.clamp(p.min, p.max),
                }
            })
            .collect()
    }

    /// Mono first; then stereo; then whatever the plugin reports for its main buses (the mono
    /// shim adapts to any channel count). Returns the final channel counts per bus.
    fn negotiate_buses(&self) -> (Vec<u32>, Vec<u32>) {
        let (n_in, n_out) = (self.buses.inputs.len(), self.buses.outputs.len());
        let current = |dir: BusDirection, n: usize, fallback: &[u32]| -> Vec<SpeakerArrangement> {
            (0..n)
                .map(|i| {
                    let mut arr: SpeakerArrangement = 0;
                    // SAFETY: inactive processor, main thread; `arr` is writable.
                    let r = unsafe { self.processor.getBusArrangement(dir, i as i32, &mut arr) };
                    if r == kResultOk {
                        arr
                    } else {
                        (1u64 << fallback[i].min(63)) - 1
                    }
                })
                .collect()
        };
        let (dir_in, dir_out) = (kInput as BusDirection, kOutput as BusDirection);
        for want in [Some(SpeakerArr::kMono), Some(SpeakerArr::kStereo), None] {
            let mut ins = current(dir_in, n_in, &self.buses.inputs);
            let mut outs = current(dir_out, n_out, &self.buses.outputs);
            if let Some(w) = want {
                ins[self.buses.main_in] = w;
                outs[self.buses.main_out] = w;
            }
            // SAFETY: inactive processor, main thread; one arrangement per bus.
            let r = unsafe {
                self.processor.setBusArrangements(
                    ins.as_mut_ptr(),
                    n_in as i32,
                    outs.as_mut_ptr(),
                    n_out as i32,
                )
            };
            if r == kResultTrue {
                break;
            }
        }
        let ins = current(dir_in, n_in, &self.buses.inputs);
        let outs = current(dir_out, n_out, &self.buses.outputs);
        (
            ins.into_iter().map(channels_of).collect(),
            outs.into_iter().map(channels_of).collect(),
        )
    }

    /// Only the main audio buses are active; event buses are off (effects only).
    fn activate_buses(&self) {
        let c = &self.component;
        for (dir, n, main) in [
            (kInput, self.buses.inputs.len(), self.buses.main_in),
            (kOutput, self.buses.outputs.len(), self.buses.main_out),
        ] {
            let dir = dir as BusDirection;
            for i in 0..n {
                // SAFETY: inactive component, main thread.
                unsafe { c.activateBus(kAudio as MediaType, dir, i as i32, u8::from(i == main)) };
            }
            // SAFETY: as above.
            for i in 0..unsafe { c.getBusCount(kEvent as MediaType, dir) } {
                // SAFETY: as above.
                unsafe { c.activateBus(kEvent as MediaType, dir, i, 0) };
            }
        }
    }

    fn build_audio(
        &self,
        max_block: usize,
        in_ch: &[u32],
        out_ch: &[u32],
        process_mode: i32,
        sample_rate: f64,
    ) -> Box<AudioState> {
        let bufs = |counts: &[u32]| -> Vec<Vec<Vec<f32>>> {
            counts
                .iter()
                .map(|&c| (0..c).map(|_| vec![0.0f32; max_block]).collect())
                .collect()
        };
        let mut in_bufs = bufs(in_ch);
        let mut out_bufs = bufs(out_ch);
        let ptrs = |b: &mut Vec<Vec<Vec<f32>>>| -> Vec<Vec<*mut f32>> {
            b.iter_mut()
                .map(|bus| bus.iter_mut().map(|ch| ch.as_mut_ptr()).collect())
                .collect()
        };
        let mut in_ptrs = ptrs(&mut in_bufs);
        let mut out_ptrs = ptrs(&mut out_bufs);
        let audio = |p: &mut Vec<Vec<*mut f32>>| -> Vec<AudioBusBuffers> {
            p.iter_mut()
                .map(|bus| AudioBusBuffers {
                    numChannels: bus.len() as i32,
                    silenceFlags: 0,
                    __field0: AudioBusBuffers__type0 {
                        channelBuffers32: bus.as_mut_ptr(),
                    },
                })
                .collect()
        };
        let in_audio = audio(&mut in_ptrs);
        let out_audio = audio(&mut out_ptrs);
        let in_changes = ParamChanges::new(
            self.params.len().clamp(1, MAX_BLOCK_PARAMS),
            MAX_BLOCK_POINTS,
        );
        let out_changes = ParamChanges::new(MAX_OUT_PARAMS, MAX_OUT_POINTS);
        // SAFETY: zeroed POD (a valid "nothing known" context), then filled below.
        let mut context: Box<ProcessContext> = Box::new(unsafe { std::mem::zeroed() });
        context.state = (kTempoValid | kTimeSigValid) as u32;
        context.sampleRate = sample_rate;
        context.tempo = 120.0;
        context.timeSigNumerator = 4;
        context.timeSigDenominator = 4;
        Box::new(AudioState {
            max_block,
            in_bufs,
            out_bufs,
            _in_ptrs: in_ptrs,
            _out_ptrs: out_ptrs,
            in_audio,
            out_audio,
            main_in: self.buses.main_in,
            main_out: self.buses.main_out,
            in_changes_ptr: changes_ptr(&in_changes),
            out_changes_ptr: changes_ptr(&out_changes),
            in_changes,
            out_changes,
            context,
            process_mode,
            steady: 0,
            processing: false,
        })
    }

    /// VST3 has no "set a parameter while inactive" call: values set since the last activation
    /// reach the processor through a zero-sample `process` under a short activation before the
    /// real one (ADR-008 Amendment 6 §2), so latency and state already reflect them.
    fn flush(&self, st: &mut AudioState, pending: &[(u32, f64)]) {
        st.in_changes.clear();
        for &(id, n) in pending {
            st.in_changes.push(id, 0, n);
        }
        let (p, c) = (&self.processor, &self.component);
        // SAFETY: main thread; the audio thread isn't running (inactive); the process data's
        // pointers stay valid for the call (no audio buses: a parameter-only call).
        unsafe {
            if c.setActive(1) != kResultOk {
                return;
            }
            p.setProcessing(1);
            let mut data = st.process_data(0);
            data.numInputs = 0;
            data.numOutputs = 0;
            data.inputs = std::ptr::null_mut();
            data.outputs = std::ptr::null_mut();
            p.process(&mut data);
            p.setProcessing(0);
            c.setActive(0);
        }
        st.in_changes.clear();
        st.out_changes.clear();
    }
}

impl PluginInstance for Vst3Instance {
    fn info(&self) -> PluginInfo {
        PluginInfo {
            name: self.name.clone(),
            vendor: self.vendor.clone(),
            version: self.version.clone(),
            params: self.params.clone(),
            groups: self.groups.clone(),
            values: self.current_values(),
            param_text: self.controller.is_some(),
        }
    }

    fn activate(&self, config: &ActivateConfig) -> Result<ActiveInfo, String> {
        let mut active = lock(&self.active);
        if *active {
            return Err("already active".into());
        }
        let max_block = config.max_block.max(1);
        let process_mode = if config.mode == ProcessMode::Offline {
            kOffline as i32
        } else {
            kRealtime as i32
        };
        let (in_ch, out_ch) = self.negotiate_buses();
        self.activate_buses();
        let p = &self.processor;
        // SAFETY: inactive processor, main thread.
        if unsafe { p.canProcessSampleSize(kSample32 as i32) } != kResultTrue {
            return Err(format!("{} can't process 32-bit audio", self.name));
        }
        let mut setup = ProcessSetup {
            processMode: process_mode,
            symbolicSampleSize: kSample32 as i32,
            maxSamplesPerBlock: max_block as i32,
            sampleRate: config.sample_rate,
        };
        // SAFETY: as above; `setup` lives for the call.
        if unsafe { p.setupProcessing(&mut setup) } != kResultOk {
            return Err(format!("{} refused the processing setup", self.name));
        }
        let mut state = self.build_audio(
            max_block as usize,
            &in_ch,
            &out_ch,
            process_mode,
            config.sample_rate,
        );
        let pending = std::mem::take(&mut *lock(&self.pending));
        if !pending.is_empty() {
            self.flush(&mut state, &pending);
        }
        // SAFETY: set up, inactive, main thread.
        if unsafe { self.component.setActive(1) } != kResultOk {
            return Err(format!("{} refused to activate", self.name));
        }
        // Requests made while being activated are covered by this activation.
        self.shared
            .restart_requested
            .store(false, Ordering::Release);
        // SAFETY: active, main thread.
        let (latency, tail) = unsafe { (p.getLatencySamples(), p.getTailSamples()) };
        *lock(&self.audio) = Some(state);
        *active = true;
        Ok(ActiveInfo {
            latency_samples: latency,
            tail_samples: (tail != kInfiniteTail).then_some(u64::from(tail)),
        })
    }

    fn deactivate(&self) {
        let mut active = lock(&self.active);
        if !*active {
            return;
        }
        // SAFETY: main thread; the audio thread has stopped (and called `setProcessing(0)`).
        unsafe { self.component.setActive(0) };
        *lock(&self.audio) = None;
        *active = false;
    }

    fn process(&self, chunk: Chunk<'_>, out_events: &mut Vec<WireEvent>) {
        let mut guard = lock(&self.audio);
        let n = chunk.input.len().min(chunk.output.len());
        let Some(st) = guard.as_deref_mut().filter(|s| n <= s.max_block) else {
            chunk.output[..n].copy_from_slice(&chunk.input[..n]);
            return;
        };
        let p = &self.processor;
        if !st.processing {
            // SAFETY: audio thread, active processor (a refusal is not fatal: many plugins
            // don't implement it).
            unsafe { p.setProcessing(1) };
            st.processing = true;
        }
        st.in_changes.clear();
        st.out_changes.clear();
        let shared = &self.shared;
        let mut push = |kind: EventKind, pos: u64, id: u32, value: f64| {
            if out_events.len() < out_events.capacity() {
                out_events.push(WireEvent {
                    pos,
                    kind,
                    id,
                    value,
                });
            }
        };
        for ev in chunk.events {
            if ev.kind == EventKind::RESET {
                // SAFETY: audio thread, active processor: the documented buffer reset.
                unsafe {
                    p.setProcessing(0);
                    p.setProcessing(1);
                }
            } else if ev.kind == EventKind::PARAM_VALUE
                && let Some(slot) = shared.slot(ev.id)
            {
                let norm = slot.domain.to_normalized(ev.value);
                st.in_changes.push(ev.id, chunk.offset(ev) as i32, norm);
                shared.note_to_controller(slot, norm);
            }
        }
        // Plugin-originated changes (its GUI's edits, `kParamValuesChanged`): to the host's
        // mirror, and edits to the processor too.
        shared.take_plugin_edits(|e| match e {
            PluginEdit::Begin(id) => push(EventKind::GESTURE_BEGIN, chunk.pos, id, 0.0),
            PluginEdit::End(id) => push(EventKind::GESTURE_END, chunk.pos, id, 0.0),
            PluginEdit::Value {
                id,
                normalized,
                deliver,
            } => {
                if let Some(slot) = shared.slot(id) {
                    if deliver {
                        st.in_changes.push(id, 0, normalized);
                    }
                    push(
                        EventKind::PARAM_VALUE,
                        chunk.pos,
                        id,
                        slot.domain.value_of(normalized),
                    );
                }
            }
        });
        // Input: the main bus's every channel carries the mono input (upmix); other buses
        // (side chains) get silence.
        for (k, bus) in st.in_bufs.iter_mut().enumerate() {
            for ch in bus.iter_mut() {
                if k == st.main_in {
                    ch[..n].copy_from_slice(&chunk.input[..n]);
                } else {
                    ch[..n].fill(0.0);
                }
            }
        }
        st.context.projectTimeSamples = st.steady;
        st.context.continousTimeSamples = st.steady;
        let mut data = st.process_data(n);
        // SAFETY: audio thread, processing plugin; every buffer holds `max_block` ≥ n frames.
        let result = unsafe { p.process(&mut data) };
        st.steady += n as i64;
        let main = &st.out_bufs[st.main_out];
        if result != kResultOk || main.is_empty() {
            chunk.output[..n].copy_from_slice(&chunk.input[..n]);
        } else if main.len() == 1 {
            chunk.output[..n].copy_from_slice(&main[0][..n]);
        } else {
            // Output: the main bus's channels averaged (downmix).
            let scale = 1.0 / main.len() as f32;
            for (i, o) in chunk.output[..n].iter_mut().enumerate() {
                let mut acc = main[0][i];
                for ch in &main[1..] {
                    acc += ch[i];
                }
                *o = acc * scale;
            }
        }
        let last = n.saturating_sub(1) as i32;
        st.out_changes.for_each(|id, offset, norm| {
            if let Some(slot) = shared.slot(id) {
                shared.note_to_controller(slot, norm);
                let pos = chunk.pos + offset.clamp(0, last) as u64;
                push(EventKind::PARAM_VALUE, pos, id, slot.domain.value_of(norm));
            }
        });
        if shared.restart_requested.swap(false, Ordering::AcqRel) {
            push(EventKind::RESTART_REQUEST, chunk.pos, 0, 0.0);
        }
    }

    fn audio_thread_stopping(&self) {
        if let Some(st) = lock(&self.audio).as_deref_mut()
            && st.processing
        {
            // SAFETY: audio thread (the audio loop calls this before it exits).
            unsafe { self.processor.setProcessing(0) };
            st.processing = false;
        }
    }

    fn main_thread_idle(&self) {
        let Some(c) = &self.controller else {
            return;
        };
        // Keep the controller in sync with what the processor got or reported.
        self.shared.take_to_controller(|id, n| {
            // SAFETY: main thread.
            unsafe { c.setParamNormalized(id, n) };
        });
        if self.shared.values_changed.swap(false, Ordering::AcqRel) {
            for p in &self.params {
                // SAFETY: main thread.
                let n = unsafe { c.getParamNormalized(p.id.0) };
                if n.is_finite() {
                    self.shared.plugin_edit(p.id.0, n, false);
                }
            }
        }
    }

    fn set_param(&self, id: ParamId, value: f64) -> Result<(), String> {
        let domain = self
            .domain(id.0)
            .ok_or_else(|| format!("no parameter {}", id.0))?;
        let n = domain.to_normalized(value);
        if let Some(c) = &self.controller {
            // SAFETY: main thread, inactive.
            let current = unsafe { c.getParamNormalized(id.0) };
            if domain.value_of(current).to_bits() == domain.value_of(n).to_bits() {
                return Ok(());
            }
            // SAFETY: as above.
            unsafe { c.setParamNormalized(id.0, n) };
        }
        let mut pending = lock(&self.pending);
        pending.retain(|(i, _)| *i != id.0);
        pending.push((id.0, n));
        Ok(())
    }

    fn save_state(&self) -> Result<Vec<u8>, String> {
        // SAFETY: main thread; the stream lives for the call.
        let component = stream::save(|s| unsafe { self.component.getState(s) })
            .map_err(|_| format!("{} couldn't save its state", self.name))?;
        let controller = match &self.controller {
            // SAFETY: as above.
            Some(c) => stream::save(|s| unsafe { c.getState(s) }).unwrap_or_default(),
            None => Vec::new(),
        };
        Ok(frame_state(&component, &controller))
    }

    fn load_state(&self, data: &[u8]) -> Result<(), String> {
        if data.is_empty() {
            return Ok(());
        }
        let (component, controller) =
            unframe_state(data).ok_or_else(|| format!("{} can't read this state", self.name))?;
        // SAFETY: main thread, inactive; the stream lives for the call.
        if stream::load(component, |s| unsafe { self.component.setState(s) }) != kResultOk {
            return Err(format!("{} rejected the state", self.name));
        }
        if let Some(c) = &self.controller {
            if self.separate {
                // SAFETY: as above.
                stream::load(component, |s| unsafe { c.setComponentState(s) });
            }
            if !controller.is_empty() {
                // SAFETY: as above (a controller that rejects its own state keeps its defaults).
                stream::load(controller, |s| unsafe { c.setState(s) });
            }
        }
        lock(&self.pending).clear();
        Ok(())
    }

    fn param_to_text(&self, id: ParamId, value: f64) -> Option<String> {
        let c = self.controller.as_ref()?;
        let n = self.domain(id.0)?.to_normalized(value);
        let text = Self::value_string(c, id.0, n)?;
        let units = self.units_of(id.0);
        Some(if units.is_empty() || text.ends_with(units) {
            text
        } else {
            format!("{text} {units}")
        })
    }

    fn text_to_param(&self, id: ParamId, text: &str) -> Option<f64> {
        let c = self.controller.as_ref()?;
        let domain = self.domain(id.0)?;
        let parse = |t: &str| -> Option<f64> {
            let mut w: Vec<TChar> = t.encode_utf16().map(|u| u as TChar).collect();
            w.push(0);
            let mut n = 0.0;
            // SAFETY: main thread; a NUL-terminated UTF-16 string; writable `n`.
            let ok = unsafe { c.getParamValueByString(id.0, w.as_mut_ptr(), &mut n) } == kResultOk;
            (ok && n.is_finite()).then(|| domain.value_of(n))
        };
        let text = text.trim();
        let units = self.units_of(id.0);
        parse(text).or_else(|| {
            let lower = text.to_lowercase();
            let stripped = (!units.is_empty() && lower.ends_with(&units.to_lowercase()))
                .then(|| text[..text.len() - units.len()].trim())?;
            parse(stripped)
        })
    }
}

impl Drop for Vst3Instance {
    fn drop(&mut self) {
        if *lock(&self.active) {
            self.deactivate();
        }
        // SAFETY: main thread; disconnect, then terminate the controller, then the component
        // (the SDK's teardown order); each exactly once.
        unsafe {
            if let Some((a, b)) = self.connection.take() {
                a.disconnect(b.as_ptr());
                b.disconnect(a.as_ptr());
            }
            if self.separate
                && let Some(c) = &self.controller
            {
                c.terminate();
            }
            self.component.terminate();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn states_are_framed_component_then_controller() {
        let blob = frame_state(b"component", b"ctl");
        assert_eq!(&blob[..4], b"PVV3");
        assert_eq!(unframe_state(&blob), Some((&b"component"[..], &b"ctl"[..])));
        assert_eq!(
            unframe_state(&frame_state(b"", b"")),
            Some((&b""[..], &b""[..]))
        );
        assert_eq!(unframe_state(b"PVV3"), None);
        assert_eq!(unframe_state(&blob[..blob.len() - 1]), None, "truncated");
        let mut extra = blob.clone();
        extra.push(0);
        assert_eq!(unframe_state(&extra), None, "trailing bytes");
        let mut v2 = blob;
        v2[4] = 2;
        assert_eq!(unframe_state(&v2), None, "unknown version");
    }
}

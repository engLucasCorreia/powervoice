//! **`vox-module-clap`** (T-805, ADR-006 §1–§2): exposes any ADR-005 [`Module`] as a standard
//! **CLAP 1.2** mono audio-effect plugin, built with `clack-plugin`. A module author writes an
//! ordinary `Module` + `ModuleFactory` and exports it from a `cdylib` crate:
//!
//! ```ignore
//! vox_module_clap::export_module!(MyFactory); // MyFactory: ModuleFactory + Default
//! ```
//!
//! The resulting `.clap` works in any CLAP host; PowerVoice recognises the
//! `org.powervoice.module-info/1` extension and treats it as a PowerVoice module (bare id,
//! key-based state, the module's own parameter schema).
//!
//! What the wrapper implements ([`ClapModule`]):
//!
//! | CLAP | From the module |
//! |---|---|
//! | descriptor | the factory's descriptor: id, name, vendor, version (semver), description, url, features |
//! | `audio-ports` | one **mono** port per direction (in-place capable); the module runs `ChannelLayout::MONO` |
//! | `params` | `ParamInfo`s: plain values, the group path as `module`, flags mapped (automatable, stepped, enum, read-only, hidden, bypass); `value_to_text`/`text_to_value` = the module API's text rules |
//! | process events | `PARAM_VALUE` events → sample-accurate `ParamEvent`s (clamped/quantized); module output events → `PARAM_VALUE` output events |
//! | `state` | UTF-8 JSON of `ModuleState` (`prepare_state` → migrations run here, on load) |
//! | `latency`, `tail` | `latency_samples()` / `tail()` read at activation; `HostRequest::Restart` → `request_restart` |
//! | `render` | realtime/offline → `ActivateConfig::mode` (applied at the next activation) |
//! | `org.powervoice.module-info/1` | `ModuleInfo` JSON (descriptor, params, groups) |
//!
//! **Live-instance rule** (ADR-005 §7, stricter than CLAP): the module has one owner at a time.
//! While the plugin is inactive it lives on the main thread; activation moves it into the audio
//! processor, deactivation moves it back. Main-thread calls made while it is active — parameter
//! values, text, state save/load — are answered from a wrapper-side **parameter mirror**
//! (atomics written by the audio thread) plus the last loaded blob (ADR-005 rule S1). A state
//! loaded while active reaches the audio thread as offset-0 parameter events; a blob in it is
//! applied at the next activation (the wrapper asks the host to restart).
//!
//! Real time: `process` doesn't allocate, lock or log (preallocated event lists and scratch
//! buffers; the mirror is atomics). A block with more parameter events than the event list holds
//! is rendered in sub-blocks, so events are never dropped.

use std::cell::{Cell, RefCell};
use std::ffi::{CStr, CString};
use std::fmt::Write as _;
use std::io::Read as _;
use std::marker::PhantomData;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

pub use clack_plugin;

use clack_extensions::audio_ports::{
    AudioPortFlags, AudioPortInfo, AudioPortInfoWriter, AudioPortType, PluginAudioPorts,
    PluginAudioPortsImpl,
};
use clack_extensions::latency::{PluginLatency, PluginLatencyImpl};
use clack_extensions::params::{
    ParamDisplayWriter, ParamInfoFlags, ParamInfoWriter, PluginAudioProcessorParams,
    PluginMainThreadParams, PluginParams,
};
use clack_extensions::render::{PluginRender, PluginRenderImpl, RenderMode};
use clack_extensions::state::{PluginState, PluginStateImpl};
use clack_extensions::tail::{PluginTail, PluginTailImpl, TailLength};
use clack_plugin::events::event_types::ParamValueEvent;
use clack_plugin::prelude::*;
use clack_plugin::stream::{InputStream, OutputStream};
use clack_plugin::utils::Cookie;
use vox_module_api::{
    ActivateConfig, ChannelLayout, DEFAULT_EVENT_CAPACITY, HostRequest, Module, ModuleDescriptor,
    ModuleFactory, ModuleInfo, ModuleState, OutputEvents as ModuleOutputEvents, ParamEvent,
    ParamFlags, ParamGroup, ParamInfo, ProcessContext, ProcessMode, ProcessStatus as ModuleStatus,
    Tail, Transport, prepare_state,
};

mod info_ext;

pub use info_ext::PluginModuleInfo;

/// Parameter events one sub-block carries (more are rendered in further sub-blocks).
const EVENT_CAPACITY: usize = DEFAULT_EVENT_CAPACITY;

/// Exports `$factory` (a `ModuleFactory + Default`) as this library's CLAP entry point
/// (`clap_entry`). Use it once, in a `cdylib` crate.
#[macro_export]
macro_rules! export_module {
    ($factory:ty) => {
        $crate::clack_plugin::clack_export_entry!(
            $crate::clack_plugin::entry::SinglePluginEntry<$crate::ClapModule<$factory>>
        );
    };
}

/// The CLAP plugin type wrapping the modules of factory `F` (see the crate docs).
pub struct ClapModule<F>(PhantomData<fn() -> F>);

/// The CLAP `flags` of a parameter.
fn clap_flags(p: &ParamInfo) -> ParamInfoFlags {
    let mut f = ParamInfoFlags::empty();
    let has = |flag| p.flags.contains(flag);
    if has(ParamFlags::AUTOMATABLE) {
        f |= ParamInfoFlags::IS_AUTOMATABLE;
    }
    if has(ParamFlags::STEPPED) || has(ParamFlags::BOOL) {
        f |= ParamInfoFlags::IS_STEPPED;
    }
    if p.is_enum() {
        f |= ParamInfoFlags::IS_ENUM | ParamInfoFlags::IS_STEPPED;
    }
    if has(ParamFlags::READ_ONLY) {
        f |= ParamInfoFlags::IS_READONLY;
    }
    if has(ParamFlags::HIDDEN) {
        f |= ParamInfoFlags::IS_HIDDEN;
    }
    if has(ParamFlags::BYPASS) {
        f |= ParamInfoFlags::IS_BYPASS;
    }
    f
}

/// `"Parent/Child"`: the group path of a parameter (CLAP's `module`), empty for the main section.
fn group_path(p: &ParamInfo, groups: &[ParamGroup]) -> String {
    let mut names = Vec::new();
    let mut next = p.group;
    // Bounded: a valid schema has no parent cycles, and the walk stops after `groups.len()`.
    for _ in 0..=groups.len() {
        let Some(id) = next else { break };
        let Some(g) = groups.iter().find(|g| g.id == id) else {
            break;
        };
        names.push(g.name.text.as_str());
        next = g.parent;
    }
    names.reverse();
    names.join("/")
}

/// State shared by every thread: the schema, the parameter mirror and the host handle.
pub struct Shared<'a, F> {
    host: HostSharedHandle<'a>,
    factory: F,
    descriptor: ModuleDescriptor,
    params: Vec<ParamInfo>,
    module_paths: Vec<String>,
    /// `(CLAP id, index into params)`, sorted by id.
    index: Vec<(u32, usize)>,
    /// Current plain value of every parameter (`f64` bits): the mirror main-thread calls read
    /// while the module is on the audio thread.
    mirror: Box<[AtomicU64]>,
    /// Set by a state load while active: the audio thread sends the mirror's value at the next
    /// block's offset 0.
    pending: Box<[AtomicBool]>,
    /// The module info JSON (`org.powervoice.module-info/1`).
    pub(crate) info_json: Vec<u8>,
    /// A restart was requested and not yet honoured (no repeated requests).
    restart_requested: AtomicBool,
}

impl<'a, F: ModuleFactory + 'a> PluginShared<'a> for Shared<'a, F> {}

impl<F> Shared<'_, F> {
    fn index_of(&self, id: u32) -> Option<usize> {
        self.index
            .binary_search_by_key(&id, |e| e.0)
            .ok()
            .map(|k| self.index[k].1)
    }

    fn value(&self, i: usize) -> f64 {
        f64::from_bits(self.mirror[i].load(Ordering::Acquire))
    }

    fn set_value(&self, i: usize, v: f64) {
        self.mirror[i].store(v.to_bits(), Ordering::Release);
    }

    /// The persistent state from the mirror (ADR-005 rule S1: parameter values plus the blob
    /// last given to `load_state`).
    fn mirror_state(&self, blob: Option<Vec<u8>>) -> ModuleState {
        ModuleState {
            format_version: self.descriptor.state_format_version,
            params: self
                .params
                .iter()
                .enumerate()
                .filter(|(_, p)| !p.flags.contains(ParamFlags::READ_ONLY))
                .map(|(i, p)| (p.key.clone(), self.value(i)))
                .collect(),
            blob,
        }
    }

    fn request_restart(&self) {
        if !self.restart_requested.swap(true, Ordering::AcqRel) {
            self.host.request_restart();
        }
    }
}

/// Main-thread state: the module while inactive, the render mode and the activation results.
pub struct MainThread<'a, F: ModuleFactory> {
    shared: &'a Shared<'a, F>,
    /// The module while the plugin is inactive (`None` while the audio processor owns it).
    live: RefCell<Option<Box<dyn Module>>>,
    mode: Cell<ProcessMode>,
    latency: Cell<u32>,
    /// The blob last given to `load_state` (rule S1).
    last_blob: RefCell<Option<Vec<u8>>>,
    /// A state was loaded while active: reload it into the module at the next activation.
    reload: Cell<bool>,
}

impl<'a, F: ModuleFactory + 'a> PluginMainThread<'a, Shared<'a, F>> for MainThread<'a, F> {}

fn module_error(e: impl std::fmt::Display) -> PluginError {
    PluginError::Error(e.to_string().into())
}

impl<F: ModuleFactory> MainThread<'_, F> {
    fn state(&self) -> Result<ModuleState, PluginError> {
        match self.live.borrow().as_deref() {
            Some(m) => m.save_state().map_err(module_error),
            None => Ok(self.shared.mirror_state(self.last_blob.borrow().clone())),
        }
    }

    fn load(&self, state: ModuleState) -> Result<(), PluginError> {
        let shared = self.shared;
        let prepared = {
            let live = self.live.borrow();
            match live.as_deref() {
                Some(m) => prepare_state(m, state),
                None => {
                    // Active: migrate against a fresh (never activated) instance.
                    let probe = shared.factory.create().map_err(module_error)?;
                    prepare_state(&*probe, state)
                }
            }
            .map_err(module_error)?
        };
        for (key, &v) in &prepared.params {
            if let Some(i) = shared.params.iter().position(|p| &p.key == key) {
                shared.set_value(i, v);
            }
        }
        *self.last_blob.borrow_mut() = prepared.blob.clone();
        let mut live = self.live.borrow_mut();
        match live.as_deref_mut() {
            Some(m) => m.load_state(&prepared).map_err(module_error)?,
            None => {
                for (flag, p) in shared.pending.iter().zip(&shared.params) {
                    if !p.flags.contains(ParamFlags::READ_ONLY) {
                        flag.store(true, Ordering::Release);
                    }
                }
                self.reload.set(true);
                if prepared.blob.is_some() {
                    // Only an activation can load a blob into the module.
                    shared.request_restart();
                }
            }
        }
        Ok(())
    }
}

impl<F: ModuleFactory> PluginMainThreadParams for MainThread<'_, F> {
    fn count(&self) -> u32 {
        self.shared.params.len() as u32
    }

    fn get_info(&self, param_index: u32, info: &mut ParamInfoWriter) {
        let shared = self.shared;
        let i = param_index as usize;
        let (Some(p), Some(path)) = (shared.params.get(i), shared.module_paths.get(i)) else {
            return;
        };
        let Some(id) = ClapId::from_raw(p.id.0) else {
            return;
        };
        info.set(&clack_extensions::params::ParamInfo {
            id,
            flags: clap_flags(p),
            cookie: Cookie::empty(),
            name: p.name.text.as_bytes(),
            module: path.as_bytes(),
            min_value: p.min,
            max_value: p.max,
            default_value: p.default,
        });
    }

    fn get_value(&self, param_id: ClapId) -> Option<f64> {
        let i = self.shared.index_of(param_id.get())?;
        Some(self.shared.value(i))
    }

    fn value_to_text(
        &self,
        param_id: ClapId,
        value: f64,
        writer: &mut ParamDisplayWriter,
    ) -> std::fmt::Result {
        let i = self
            .shared
            .index_of(param_id.get())
            .ok_or(std::fmt::Error)?;
        writer.write_str(&self.shared.params[i].value_to_text(value))
    }

    fn text_to_value(&self, param_id: ClapId, text: &CStr) -> Option<f64> {
        let i = self.shared.index_of(param_id.get())?;
        self.shared.params[i].text_to_value(text.to_str().ok()?)
    }

    /// Inactive (CLAP calls the audio-thread flush while active): the values go to the mirror,
    /// then into the module through `load_state` (the only way to set values of an inactive
    /// module).
    fn flush(&self, input: &InputEvents, _output: &mut OutputEvents) {
        let shared = self.shared;
        let mut changed = false;
        for ev in input {
            let Some(pv) = ev.as_event::<ParamValueEvent>() else {
                continue;
            };
            let Some(i) = pv.param_id().and_then(|id| shared.index_of(id.get())) else {
                continue;
            };
            let p = &shared.params[i];
            if !p.flags.contains(ParamFlags::READ_ONLY) {
                shared.set_value(i, p.clamp_quantize(pv.value()));
                changed = true;
            }
        }
        if changed && let Some(m) = self.live.borrow_mut().as_deref_mut() {
            let _ = m.load_state(&shared.mirror_state(self.last_blob.borrow().clone()));
        }
    }
}

impl<F: ModuleFactory> PluginStateImpl for MainThread<'_, F> {
    fn save(&self, output: &mut OutputStream) -> Result<(), PluginError> {
        let state = self.state()?;
        serde_json::to_writer(output, &state)?;
        Ok(())
    }

    fn load(&self, input: &mut InputStream) -> Result<(), PluginError> {
        let mut bytes = Vec::new();
        input.read_to_end(&mut bytes)?;
        let state: ModuleState = serde_json::from_slice(&bytes)?;
        MainThread::load(self, state)
    }
}

impl<F: ModuleFactory> PluginLatencyImpl for MainThread<'_, F> {
    fn get(&self) -> u32 {
        self.latency.get()
    }
}

impl<F: ModuleFactory> PluginRenderImpl for MainThread<'_, F> {
    fn has_hard_realtime_requirement(&self) -> bool {
        false
    }

    fn set(&self, mode: RenderMode) -> Result<(), PluginError> {
        let mode = match mode {
            RenderMode::Offline => ProcessMode::Offline,
            _ => ProcessMode::Realtime,
        };
        if mode != self.mode.get() {
            self.mode.set(mode);
            if self.live.borrow().is_none() {
                // Active: the module's mode is fixed until it is activated again.
                self.shared.request_restart();
            }
        }
        Ok(())
    }
}

impl<F: ModuleFactory> PluginAudioPortsImpl for MainThread<'_, F> {
    fn count(&self, _is_input: bool) -> u32 {
        1
    }

    fn get(&self, index: u32, _is_input: bool, writer: &mut AudioPortInfoWriter) {
        if index == 0 {
            writer.set(&AudioPortInfo {
                id: ClapId::new(0),
                name: b"main",
                channel_count: 1,
                flags: AudioPortFlags::IS_MAIN,
                port_type: Some(AudioPortType::MONO),
                in_place_pair: Some(ClapId::new(0)),
            });
        }
    }
}

/// The audio processor: the active module, its event list and scratch buffers.
pub struct Processor<'a, F> {
    shared: &'a Shared<'a, F>,
    module: Box<dyn Module>,
    events: Vec<ParamEvent>,
    out: ModuleOutputEvents,
    /// In-place input copy / silence for an output-only port, `max_frames` long.
    scratch: Vec<f32>,
    /// Output sink for an input-only port, `max_frames` long.
    sink: Vec<f32>,
    tail: TailLength,
    steady: u64,
}

impl<F: ModuleFactory> Processor<'_, F> {
    /// Pending changes of a state loaded while active: offset-0 events.
    fn take_pending(&mut self) {
        let shared = self.shared;
        for (i, flag) in shared.pending.iter().enumerate() {
            if self.events.len() < self.events.capacity() && flag.swap(false, Ordering::AcqRel) {
                self.events.push(ParamEvent {
                    offset: 0,
                    id: shared.params[i].id,
                    value: shared.value(i),
                });
            }
        }
    }

    /// The module event for a host `PARAM_VALUE` event (mirror updated), if it's one of ours.
    fn param_event(&self, ev: &UnknownEvent) -> Option<(u32, ParamEvent)> {
        let shared = self.shared;
        let pv = ev.as_event::<ParamValueEvent>()?;
        let i = shared.index_of(pv.param_id()?.get())?;
        let p = &shared.params[i];
        if p.flags.contains(ParamFlags::READ_ONLY) {
            return None;
        }
        let value = p.clamp_quantize(pv.value());
        shared.set_value(i, value);
        Some((
            pv.header().time(),
            ParamEvent {
                offset: 0,
                id: p.id,
                value,
            },
        ))
    }

    /// Renders `[start, end)` of `input` into `output` with the collected events (offsets
    /// relative to `start`), forwards the module's output events, clears the event list.
    fn render(
        &mut self,
        start: u32,
        end: u32,
        input: &[f32],
        output: &mut [f32],
        out_events: &mut OutputEvents,
    ) -> ModuleStatus {
        let shared = self.shared;
        let (s, e) = (start as usize, end as usize);
        let Self {
            module,
            events,
            out,
            steady,
            ..
        } = self;
        let mut ctx = ProcessContext::new(
            end - start,
            *steady + u64::from(start),
            Transport::default(),
            events,
            out,
        );
        let status = module.process(&mut ctx, &[&input[s..e]], &mut [&mut output[s..e]]);
        let restart = ctx.requested(HostRequest::Restart);
        events.clear();
        if restart {
            shared.request_restart();
        }
        for ev in out.as_slice() {
            if let Some(i) = shared.index_of(ev.id.0) {
                shared.set_value(i, ev.value);
            }
            if let Some(id) = ClapId::from_raw(ev.id.0) {
                let time = start + ev.offset.min(end - start);
                let _ = out_events.try_push(ParamValueEvent::new(
                    time,
                    id,
                    Pckn::match_all(),
                    ev.value,
                ));
            }
        }
        out.clear();
        status
    }

    /// One mono block: host events split into sub-blocks when the event list fills.
    fn run(
        &mut self,
        input: &[f32],
        output: &mut [f32],
        events: Events<'_>,
    ) -> Result<(), PluginError> {
        let frames = input.len().min(output.len()) as u32;
        self.events.clear();
        self.take_pending();
        let mut failed = false;
        let mut start = 0u32;
        for ev in events.input {
            let Some((time, mut pe)) = self.param_event(ev) else {
                continue;
            };
            let t = time.clamp(start, frames);
            if self.events.len() == self.events.capacity() {
                failed |=
                    self.render(start, t, input, output, events.output) == ModuleStatus::Error;
                start = t;
            }
            pe.offset = t - start;
            self.events.push(pe);
        }
        failed |= self.render(start, frames, input, output, events.output) == ModuleStatus::Error;
        self.steady += u64::from(frames);
        if failed {
            return Err(PluginError::Message("the module failed to process"));
        }
        Ok(())
    }
}

impl<'a, F: ModuleFactory + 'a> PluginAudioProcessor<'a, Shared<'a, F>, MainThread<'a, F>>
    for Processor<'a, F>
{
    fn activate(
        _host: HostAudioProcessorHandle<'a>,
        main_thread: &MainThread<'a, F>,
        shared: &'a Shared<'a, F>,
        audio_config: PluginAudioConfiguration,
    ) -> Result<Self, PluginError> {
        let mut module = main_thread
            .live
            .borrow_mut()
            .take()
            .ok_or(PluginError::Message("already active"))?;
        let restore = |module| *main_thread.live.borrow_mut() = Some(module);
        if main_thread.reload.get() {
            let state = shared.mirror_state(main_thread.last_blob.borrow().clone());
            if let Err(e) = module.load_state(&state) {
                restore(module);
                return Err(module_error(e));
            }
            main_thread.reload.set(false);
            for flag in shared.pending.iter() {
                flag.store(false, Ordering::Release);
            }
        }
        let max_frames = audio_config.max_frames_count.max(1);
        let config = ActivateConfig {
            sample_rate: audio_config.sample_rate,
            max_block: max_frames,
            mode: main_thread.mode.get(),
            layout: ChannelLayout::MONO,
        };
        if let Err(e) = module.activate(&config) {
            restore(module);
            return Err(module_error(e));
        }
        main_thread.latency.set(module.latency_samples());
        let tail = match module.tail() {
            Tail::Samples(n) => TailLength::Finite(u32::try_from(n).unwrap_or(u32::MAX)),
            Tail::Infinite => TailLength::Infinite,
        };
        shared.restart_requested.store(false, Ordering::Release);
        Ok(Self {
            shared,
            module,
            events: Vec::with_capacity(EVENT_CAPACITY),
            out: ModuleOutputEvents::with_capacity(EVENT_CAPACITY),
            scratch: vec![0.0; max_frames as usize],
            sink: vec![0.0; max_frames as usize],
            tail,
            steady: 0,
        })
    }

    fn process(
        &mut self,
        process: Process,
        mut audio: Audio,
        events: Events,
    ) -> Result<ProcessStatus, PluginError> {
        if let Some(t) = process.steady_time {
            self.steady = t;
        }
        let frames = audio.frames_count() as usize;
        if frames > self.scratch.len() {
            return Err(PluginError::Message("block longer than max_frames_count"));
        }
        let Some(mut pair) = audio.port_pair(0) else {
            return Ok(ProcessStatus::Continue);
        };
        let channels = pair
            .channels()
            .map_err(|_| PluginError::Message("unusable audio buffers"))?;
        let Some(mut channels) = channels.into_f32() else {
            return Err(PluginError::Message("64-bit audio isn't supported"));
        };
        let Some(pair) = channels.channel_pair(0) else {
            return Ok(ProcessStatus::Continue);
        };
        // The scratch buffers are moved out for the call (a moved-out `Vec` leaves an empty one
        // behind: no allocation) so the module and the buffers can be borrowed together.
        let mut scratch = std::mem::take(&mut self.scratch);
        let mut sink = std::mem::take(&mut self.sink);
        let result = match pair {
            ChannelPair::InputOutput(input, output) => {
                let n = frames.min(input.len()).min(output.len());
                self.run(&input[..n], &mut output[..n], events)
            }
            ChannelPair::InPlace(buf) => {
                let n = frames.min(buf.len());
                scratch[..n].copy_from_slice(&buf[..n]);
                self.run(&scratch[..n], &mut buf[..n], events)
            }
            ChannelPair::OutputOnly(output) => {
                let n = frames.min(output.len());
                scratch[..n].fill(0.0);
                self.run(&scratch[..n], &mut output[..n], events)
            }
            ChannelPair::InputOnly(input) => {
                let n = frames.min(input.len());
                self.run(&input[..n], &mut sink[..n], events)
            }
        };
        self.scratch = scratch;
        self.sink = sink;
        result.map(|()| ProcessStatus::Continue)
    }

    fn deactivate(self, main_thread: &MainThread<'a, F>) {
        let mut module = self.module;
        module.deactivate();
        *main_thread.live.borrow_mut() = Some(module);
    }

    fn reset(&mut self) {
        self.module.reset();
    }
}

impl<F: ModuleFactory> PluginAudioProcessorParams for Processor<'_, F> {
    /// Active, not processing: the events go through a zero-frame `process` (a legal parameter
    /// flush, ADR-005 §6).
    fn flush(&mut self, input: &InputEvents, output: &mut OutputEvents) {
        self.events.clear();
        self.take_pending();
        for ev in input {
            if self.events.len() == self.events.capacity() {
                let _ = self.render(0, 0, &[], &mut [], output);
            }
            if let Some((_, pe)) = self.param_event(ev) {
                self.events.push(pe);
            }
        }
        let _ = self.render(0, 0, &[], &mut [], output);
    }
}

impl<F: ModuleFactory> PluginTailImpl for Processor<'_, F> {
    fn get(&self) -> TailLength {
        self.tail
    }
}

impl<F: ModuleFactory + Default + 'static> Plugin for ClapModule<F> {
    type AudioProcessor<'a> = Processor<'a, F>;
    type Shared<'a> = Shared<'a, F>;
    type MainThread<'a> = MainThread<'a, F>;

    fn declare_extensions(builder: &mut PluginExtensions<Self>, _shared: Option<&Shared<'_, F>>) {
        builder
            .register::<PluginAudioPorts>()
            .register::<PluginParams>()
            .register::<PluginState>()
            .register::<PluginLatency>()
            .register::<PluginTail>()
            .register::<PluginRender>()
            .register::<PluginModuleInfo>();
    }
}

impl<F: ModuleFactory + Default + 'static> DefaultPluginFactory for ClapModule<F> {
    fn get_descriptor() -> PluginDescriptor {
        let factory = F::default();
        let d = factory.descriptor();
        let name = if d.name.text.is_empty() {
            &d.id
        } else {
            &d.name.text
        };
        let features: Vec<CString> = d
            .features
            .iter()
            .filter_map(|f| CString::new(f.as_str()).ok())
            .collect();
        PluginDescriptor::new(&d.id, name)
            .with_vendor(&d.vendor)
            .with_version(&d.version.to_string())
            .with_description(&d.description.text)
            .with_url(d.url.as_deref().unwrap_or(""))
            .with_features(features.iter().map(CString::as_c_str))
    }

    fn new_shared(host: HostSharedHandle<'_>) -> Result<Shared<'_, F>, PluginError> {
        let factory = F::default();
        let descriptor = factory.descriptor().clone();
        let probe = factory.create().map_err(module_error)?;
        let info = ModuleInfo::of(descriptor.clone(), &*probe);
        info.validate().map_err(module_error)?;
        let params = info.params.clone();
        let module_paths = params.iter().map(|p| group_path(p, &info.groups)).collect();
        let mut index: Vec<(u32, usize)> = params
            .iter()
            .enumerate()
            .map(|(i, p)| (p.id.0, i))
            .collect();
        index.sort_unstable();
        let mirror = params
            .iter()
            .map(|p| AtomicU64::new(probe.param_value(p.id).unwrap_or(p.default).to_bits()))
            .collect();
        let pending = params.iter().map(|_| AtomicBool::new(false)).collect();
        let info_json = serde_json::to_vec(&info)?;
        Ok(Shared {
            host,
            factory,
            descriptor,
            params,
            module_paths,
            index,
            mirror,
            pending,
            info_json,
            restart_requested: AtomicBool::new(false),
        })
    }

    fn new_main_thread<'a>(
        _host: HostMainThreadHandle<'a>,
        shared: &'a Shared<'a, F>,
    ) -> Result<MainThread<'a, F>, PluginError> {
        let live = shared.factory.create().map_err(module_error)?;
        Ok(MainThread {
            shared,
            live: RefCell::new(Some(live)),
            mode: Cell::new(ProcessMode::Realtime),
            latency: Cell::new(0),
            last_blob: RefCell::new(None),
            reload: Cell::new(false),
        })
    }
}

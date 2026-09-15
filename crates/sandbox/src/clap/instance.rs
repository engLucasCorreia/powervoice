//! One CLAP plugin as a sandbox [`PluginInstance`].

use std::ffi::{CString, c_char, c_ulong, c_void};
use std::path::Path;
use std::sync::atomic::Ordering;
use std::sync::{Mutex, MutexGuard, PoisonError};

use vox_clap_abi::*;
use vox_module_api::{ActivateConfig, ParamGroup, ParamId, ParamInfo};
use vox_sandbox_ipc::protocol::{ParamValue, PluginInfo};
use vox_sandbox_ipc::{Chunk, EventKind, WireEvent};

use super::host::{HostBox, NO_RESIZE, mark_audio_thread};
use super::library::ClapLibrary;
use super::params::{self, RawParam};
use super::stream;
use crate::backend::{ActiveInfo, MainThreadEvents, OUT_EVENT_CAPACITY, PluginInstance};
use crate::gui::{Api, EditorHost, NativeWindow, runloop};

/// Parameter events one chunk forwards at most (more are dropped; the segment's event ring is
/// smaller anyway).
const MAX_CHUNK_EVENTS: usize = 1024;
/// `value_to_text` buffer.
const TEXT_CAPACITY: usize = 256;

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The plugin's extension vtables (null when absent).
struct Ext {
    params: *const clap_plugin_params,
    state: *const clap_plugin_state,
    latency: *const clap_plugin_latency,
    tail: *const clap_plugin_tail,
    ports: *const clap_plugin_audio_ports,
    /// T-901.
    gui: *const clap_plugin_gui,
}

/// Channel counts of the plugin's audio ports and which one is main.
#[derive(Clone, Debug, Default)]
struct Ports {
    inputs: Vec<u32>,
    outputs: Vec<u32>,
    main_in: usize,
    main_out: usize,
}

/// A plugin-originated event of one chunk (offset, kind, id, value).
type OutEvent = (u32, EventKind, u32, f64);

/// The audio thread's working set, built at activation (heap buffers: the pointers stay valid).
struct AudioState {
    max_block: usize,
    in_bufs: Vec<Vec<Vec<f32>>>,
    out_bufs: Vec<Vec<Vec<f32>>>,
    /// Kept alive for `in_audio`/`out_audio`'s `data32`.
    _in_ptrs: Vec<Vec<*mut f32>>,
    _out_ptrs: Vec<Vec<*mut f32>>,
    in_audio: Vec<clap_audio_buffer>,
    out_audio: Vec<clap_audio_buffer>,
    events: Vec<clap_event_param_value>,
    out_events: Vec<OutEvent>,
    steady: i64,
    processing: bool,
    start_failed: bool,
}

// SAFETY: the raw pointers point into this struct's own heap buffers; the state moves between
// the main thread (activation) and the audio thread (processing) behind a mutex.
unsafe impl Send for AudioState {}

/// A loaded CLAP plugin (ADR-008 Amendment 3).
pub struct ClapInstance {
    plugin: *const clap_plugin,
    ext: Ext,
    name: String,
    vendor: String,
    version: String,
    params: Vec<ParamInfo>,
    groups: Vec<ParamGroup>,
    /// A PowerVoice module's checked `org.powervoice.module-info/1` JSON (T-805; `None`: an
    /// ordinary CLAP plugin). Its schema replaces the generic CLAP mapping.
    module_info: Option<String>,
    /// `(id, cookie)` sorted by id, for the events `process` builds.
    cookies: Vec<(u32, usize)>,
    ports: Ports,
    active: Mutex<bool>,
    audio: Mutex<Option<Box<AudioState>>>,
    /// T-901: the editor is open (`Some(floating)`).
    gui_open: Mutex<Option<bool>>,
    host: Box<HostBox>,
    /// Dropped last: `deinit` and unload after the plugin was destroyed.
    _library: ClapLibrary,
}

// SAFETY: CLAP's threading contract is kept by the sandbox: `[main-thread]` calls come from
// the control loop's thread only, `[audio-thread]` calls from the audio thread only (the audio
// state is behind a mutex), and `plugin`/`ext` stay valid until `Drop`.
unsafe impl Send for ClapInstance {}
// SAFETY: see `Send`.
unsafe impl Sync for ClapInstance {}

unsafe extern "C" fn in_size(list: *const clap_input_events) -> u32 {
    // SAFETY: `ctx` is the event vector of the running call.
    let v = unsafe { &*((*list).ctx as *const Vec<clap_event_param_value>) };
    v.len() as u32
}

unsafe extern "C" fn in_get(
    list: *const clap_input_events,
    index: u32,
) -> *const clap_event_header {
    // SAFETY: `ctx` is the event vector of the running call.
    let v = unsafe { &*((*list).ctx as *const Vec<clap_event_param_value>) };
    v.get(index as usize)
        .map_or(std::ptr::null(), |e| &raw const e.header)
}

unsafe extern "C" fn out_try_push(
    list: *const clap_output_events,
    ev: *const clap_event_header,
) -> bool {
    if ev.is_null() {
        return false;
    }
    // SAFETY: `ctx` is the out-event vector of the running call; `ev` a valid event.
    let (out, h) = unsafe { (&mut *((*list).ctx as *mut Vec<OutEvent>), &*ev) };
    if h.space_id != CLAP_CORE_EVENT_SPACE_ID {
        return true;
    }
    let item = match h.type_ {
        CLAP_EVENT_PARAM_VALUE => {
            // SAFETY: the type says so.
            let e = unsafe { &*(ev as *const clap_event_param_value) };
            (h.time, EventKind::PARAM_VALUE, e.param_id, e.value)
        }
        CLAP_EVENT_PARAM_GESTURE_BEGIN | CLAP_EVENT_PARAM_GESTURE_END => {
            // SAFETY: the type says so.
            let e = unsafe { &*(ev as *const clap_event_param_gesture) };
            let kind = if h.type_ == CLAP_EVENT_PARAM_GESTURE_BEGIN {
                EventKind::GESTURE_BEGIN
            } else {
                EventKind::GESTURE_END
            };
            (h.time, kind, e.param_id, 0.0)
        }
        _ => return true,
    };
    if out.len() >= out.capacity() {
        return false;
    }
    out.push(item);
    true
}

fn param_event(time: u32, id: u32, cookie: *mut c_void, value: f64) -> clap_event_param_value {
    clap_event_param_value {
        header: clap_event_header {
            size: std::mem::size_of::<clap_event_param_value>() as u32,
            time,
            space_id: CLAP_CORE_EVENT_SPACE_ID,
            type_: CLAP_EVENT_PARAM_VALUE,
            flags: 0,
        },
        param_id: id,
        cookie,
        note_id: -1,
        port_index: -1,
        channel: -1,
        key: -1,
        value,
    }
}

impl ClapInstance {
    /// Loads plugin `id` from `path` (main thread: the caller's thread becomes the plugin's
    /// main thread).
    pub fn load(path: &Path, id: &str) -> Result<Self, String> {
        let library = ClapLibrary::open(path)?;
        let factory = library
            .plugin_factory()
            .ok_or_else(|| format!("{} has no CLAP plugin factory", path.display()))?;
        let (Some(count), Some(get), Some(create)) = (
            factory.get_plugin_count,
            factory.get_plugin_descriptor,
            factory.create_plugin,
        ) else {
            return Err(format!(
                "{} has an incomplete plugin factory",
                path.display()
            ));
        };
        let mut found = None;
        // SAFETY: the factory of an initialised library, main thread.
        for i in 0..unsafe { count(factory) } {
            // SAFETY: `i < count`; descriptors live as long as the library.
            let d = unsafe { get(factory, i) };
            // SAFETY: a valid descriptor's NUL-terminated strings (or null).
            if !d.is_null() && unsafe { opt_str((*d).id) }.as_deref() == Some(id) {
                // SAFETY: as above.
                found = Some(unsafe {
                    (
                        opt_str((*d).name).unwrap_or_else(|| id.to_owned()),
                        opt_str((*d).vendor).unwrap_or_default(),
                        opt_str((*d).version).unwrap_or_default(),
                    )
                });
                break;
            }
        }
        let (name, vendor, version) =
            found.ok_or_else(|| format!("{} has no plugin `{id}`", path.display()))?;
        let host = HostBox::new(&name);
        let c_id = CString::new(id).map_err(|_| format!("bad plugin id `{id}`"))?;
        // SAFETY: the factory creates a plugin for our host (which outlives it).
        let plugin = unsafe { create(factory, &raw const host.clap, c_id.as_ptr()) };
        if plugin.is_null() {
            return Err(format!("{name} couldn't be created"));
        }
        // T-901: timer and descriptor callbacks call into the plugin.
        host.state
            .plugin
            .store(plugin.cast_mut(), Ordering::Release);
        let mut me = Self {
            plugin,
            ext: Ext {
                params: std::ptr::null(),
                state: std::ptr::null(),
                latency: std::ptr::null(),
                tail: std::ptr::null(),
                ports: std::ptr::null(),
                gui: std::ptr::null(),
            },
            name,
            vendor,
            version,
            params: Vec::new(),
            groups: Vec::new(),
            module_info: None,
            cookies: Vec::new(),
            ports: Ports::default(),
            active: Mutex::new(false),
            audio: Mutex::new(None),
            gui_open: Mutex::new(None),
            host,
            _library: library,
        };
        // From here on, `Drop` destroys the plugin on every error path.
        // SAFETY: a freshly created plugin: `init` once, on the main thread.
        if !unsafe { me.p().init.is_some_and(|f| f(plugin)) } {
            return Err(format!("{} failed to initialise", me.name));
        }
        me.ext = Ext {
            params: me.extension(CLAP_EXT_PARAMS).cast(),
            state: me.extension(CLAP_EXT_STATE).cast(),
            latency: me.extension(CLAP_EXT_LATENCY).cast(),
            tail: me.extension(CLAP_EXT_TAIL).cast(),
            ports: me.extension(CLAP_EXT_AUDIO_PORTS).cast(),
            gui: me.extension(CLAP_EXT_GUI).cast(),
        };
        me.ports = me.read_ports()?;
        let raw = me.read_params();
        let (params, groups) = match me.read_module_info(id, &raw)? {
            // A PowerVoice module (ADR-006 §2): its own schema — keys, units, tapers, i18n keys.
            Some((info, json)) => {
                me.module_info = Some(json);
                (info.params, info.groups)
            }
            None => params::map(&raw, |id, v| me.value_text(id, v)),
        };
        me.params = params;
        me.groups = groups;
        Ok(me)
    }

    /// The plugin's `org.powervoice.module-info/1` (T-805), checked against its CLAP parameters
    /// `raw`: `Ok(None)` for an ordinary plugin; an error (the plugin is refused) when it has the
    /// extension but the JSON is invalid or disagrees with `params`.
    fn read_module_info(
        &self,
        id: &str,
        raw: &[RawParam],
    ) -> Result<Option<(vox_module_api::ModuleInfo, String)>, String> {
        let ext = self.extension(POWERVOICE_EXT_MODULE_INFO) as *const clap_plugin_module_info;
        // SAFETY: null or the plugin's vtable for this id, valid while the plugin lives.
        let Some(ext) = (unsafe { ext.as_ref() }) else {
            return Ok(None);
        };
        let refuse =
            |why: String| format!("{} has invalid PowerVoice module info: {why}", self.name);
        let get = ext.get_info.ok_or_else(|| refuse("no get_info".into()))?;
        // SAFETY: main thread; the stream lives for the call.
        let json = stream::save(|s| unsafe { get(self.plugin, s) })
            .ok_or_else(|| refuse("get_info failed".into()))?;
        let info = super::module_info::check(&json, id, raw).map_err(refuse)?;
        let normalized = serde_json::to_string(&info).map_err(|e| refuse(e.to_string()))?;
        Ok(Some((info, normalized)))
    }

    /// The checked module info JSON of a PowerVoice module (T-805), `None` for a plain plugin.
    pub(crate) fn module_info(&self) -> Option<&str> {
        self.module_info.as_deref()
    }

    fn p(&self) -> &clap_plugin {
        // SAFETY: valid from creation until `Drop`.
        unsafe { &*self.plugin }
    }

    /// Parameter count (T-804 richer scan data, ADR-008 §6).
    pub(crate) fn param_count(&self) -> u32 {
        self.params.len() as u32
    }

    /// `(main input channels, main output channels)` (T-804 richer scan data, ADR-008 §6): `0`
    /// when the plugin has no port on that side (never true for a loaded audio effect, since
    /// [`Self::load`] requires both).
    pub(crate) fn main_ports(&self) -> (u32, u32) {
        (
            self.ports
                .inputs
                .get(self.ports.main_in)
                .copied()
                .unwrap_or(0),
            self.ports
                .outputs
                .get(self.ports.main_out)
                .copied()
                .unwrap_or(0),
        )
    }

    fn extension(&self, id: &std::ffi::CStr) -> *const c_void {
        // SAFETY: `get_extension` is thread-safe; `id` is NUL-terminated.
        unsafe {
            self.p()
                .get_extension
                .map_or(std::ptr::null(), |f| f(self.plugin, id.as_ptr()))
        }
    }

    fn read_ports(&self) -> Result<Ports, String> {
        if self.ext.ports.is_null() {
            return Err(format!(
                "{} has no audio ports (not an audio effect)",
                self.name
            ));
        }
        // SAFETY: the plugin's `audio-ports` vtable; main thread, inactive.
        let ext = unsafe { &*self.ext.ports };
        let (Some(count), Some(get)) = (ext.count, ext.get) else {
            return Err(format!(
                "{} has an incomplete audio-ports extension",
                self.name
            ));
        };
        let mut ports = Ports::default();
        for is_input in [true, false] {
            let mut channels = Vec::new();
            let mut main = None;
            // SAFETY: as above.
            for i in 0..unsafe { count(self.plugin, is_input) } {
                // SAFETY: zeroed POD the plugin fills.
                let mut info: clap_audio_port_info = unsafe { std::mem::zeroed() };
                // SAFETY: `i < count`; `info` is writable.
                if !unsafe { get(self.plugin, i, is_input, &mut info) } {
                    return Err(format!("{} failed to describe its audio ports", self.name));
                }
                if main.is_none()
                    && info.flags & CLAP_AUDIO_PORT_IS_MAIN != 0
                    && info.channel_count > 0
                {
                    main = Some(channels.len());
                }
                channels.push(info.channel_count);
            }
            let main = main.or_else(|| channels.iter().position(|&c| c > 0));
            let Some(main) = main else {
                return Err(format!(
                    "{} has no audio {} (not an audio effect)",
                    self.name,
                    if is_input { "input" } else { "output" }
                ));
            };
            if is_input {
                (ports.inputs, ports.main_in) = (channels, main);
            } else {
                (ports.outputs, ports.main_out) = (channels, main);
            }
        }
        Ok(ports)
    }

    fn read_params(&mut self) -> Vec<RawParam> {
        let Some(ext) = self.params_ext() else {
            return Vec::new();
        };
        let (Some(count), Some(get)) = (ext.count, ext.get_info) else {
            return Vec::new();
        };
        let mut raw = Vec::new();
        // SAFETY: main thread, the plugin's `params` vtable.
        for i in 0..unsafe { count(self.plugin) } {
            // SAFETY: zeroed POD the plugin fills.
            let mut info: clap_param_info = unsafe { std::mem::zeroed() };
            // SAFETY: `i < count`; `info` is writable.
            if !unsafe { get(self.plugin, i, &mut info) } {
                continue;
            }
            self.cookies.push((info.id, info.cookie as usize));
            raw.push(RawParam {
                id: info.id,
                flags: info.flags,
                name: fixed_str(&info.name),
                module: fixed_str(&info.module),
                min: info.min_value,
                max: info.max_value,
                default: info.default_value,
            });
        }
        self.cookies.sort_unstable_by_key(|c| c.0);
        self.cookies.dedup_by_key(|c| c.0);
        raw
    }

    fn params_ext(&self) -> Option<&clap_plugin_params> {
        // SAFETY: null or the plugin's vtable, valid while the plugin lives.
        (!self.ext.params.is_null()).then(|| unsafe { &*self.ext.params })
    }

    fn cookie(&self, id: u32) -> *mut c_void {
        self.cookies
            .binary_search_by_key(&id, |c| c.0)
            .map_or(std::ptr::null_mut(), |i| self.cookies[i].1 as *mut c_void)
    }

    fn value_text(&self, id: u32, value: f64) -> Option<String> {
        let f = self.params_ext()?.value_to_text?;
        let mut buf = [0 as c_char; TEXT_CAPACITY];
        // SAFETY: main thread; `buf` has `TEXT_CAPACITY` chars.
        let ok = unsafe {
            f(
                self.plugin,
                id,
                value,
                buf.as_mut_ptr(),
                TEXT_CAPACITY as u32,
            )
        };
        buf[TEXT_CAPACITY - 1] = 0;
        ok.then(|| fixed_str(&buf)).filter(|s| !s.is_empty())
    }

    fn current_values(&self) -> Vec<ParamValue> {
        let get = self.params_ext().and_then(|e| e.get_value);
        self.params
            .iter()
            .map(|p| {
                let mut v = p.default;
                if let Some(f) = get {
                    let mut out = 0.0;
                    // SAFETY: main thread; `out` is writable.
                    if unsafe { f(self.plugin, p.id.0, &mut out) } && out.is_finite() {
                        v = out.clamp(p.min, p.max);
                    }
                }
                ParamValue { id: p.id, value: v }
            })
            .collect()
    }

    /// `params.flush` with `events` (main thread, inactive); returns what the plugin reported.
    fn flush(&self, mut events: Vec<clap_event_param_value>) -> Vec<OutEvent> {
        let Some(flush) = self.params_ext().and_then(|e| e.flush) else {
            return Vec::new();
        };
        let mut sink: Vec<OutEvent> = Vec::with_capacity(64);
        let in_list = clap_input_events {
            ctx: (&raw mut events).cast(),
            size: Some(in_size),
            get: Some(in_get),
        };
        let out_list = clap_output_events {
            ctx: (&raw mut sink).cast(),
            try_push: Some(out_try_push),
        };
        // SAFETY: inactive → main thread; the lists live for the call.
        unsafe { flush(self.plugin, &in_list, &out_list) };
        sink
    }

    // --- T-901: the editor ---------------------------------------------------------------

    fn gui_ext(&self) -> Option<&clap_plugin_gui> {
        // SAFETY: null or the plugin's vtable, valid while the plugin lives.
        unsafe { self.ext.gui.as_ref() }
    }

    fn gui_size(&self, gui: &clap_plugin_gui) -> Option<(u32, u32)> {
        let get = gui.get_size?;
        let (mut w, mut h) = (0u32, 0u32);
        // SAFETY: main thread, a created GUI; writable outputs.
        (unsafe { get(self.plugin, &mut w, &mut h) } && w > 0 && h > 0).then_some((w, h))
    }

    fn gui_resizable(&self, gui: &clap_plugin_gui) -> bool {
        // SAFETY: main thread, a created GUI.
        gui.can_resize.is_some_and(|f| unsafe { f(self.plugin) })
    }

    /// Embedded: our top-level window at the plugin's size, the GUI attached, shown.
    fn attach_embedded(
        &self,
        gui: &clap_plugin_gui,
        host: &mut EditorHost<'_>,
    ) -> Result<(u32, u32), String> {
        let (w, h) = self.gui_size(gui).unwrap_or((640, 400));
        let window = host.create_window(w, h, self.gui_resizable(gui))?;
        let cw = clap_window_of(window);
        let set_parent = gui
            .set_parent
            .ok_or_else(|| format!("{} can't be embedded in a window", self.name))?;
        // SAFETY: main thread, a created embedded GUI; `cw` lives for the call.
        if !unsafe { set_parent(self.plugin, &cw) } {
            return Err(format!("{} couldn't attach its window", self.name));
        }
        // SAFETY: as above.
        unsafe {
            if let Some(show) = gui.show {
                show(self.plugin);
            }
        }
        Ok((w, h))
    }

    /// Floating: the plugin's own window, above the editor's where the platform allows.
    fn show_floating(
        &self,
        gui: &clap_plugin_gui,
        host: &mut EditorHost<'_>,
    ) -> Result<(u32, u32), String> {
        if let (Some(t), Some(set_transient)) = (host.transient_for(), gui.set_transient) {
            let cw = clap_window_of(t);
            // SAFETY: main thread, a created floating GUI; `cw` lives for the call.
            unsafe { set_transient(self.plugin, &cw) };
        }
        if let Some(suggest) = gui.suggest_title {
            let title = CString::new(host.title().replace('\0', "")).unwrap_or_default();
            // SAFETY: as above; NUL-terminated title.
            unsafe { suggest(self.plugin, title.as_ptr()) };
        }
        let show = gui
            .show
            .ok_or_else(|| format!("{} can't show its window", self.name))?;
        // SAFETY: as above.
        if !unsafe { show(self.plugin) } {
            return Err(format!("{} couldn't show its window", self.name));
        }
        Ok(self.gui_size(gui).unwrap_or((0, 0)))
    }

    fn build_audio(&self, max_block: usize) -> Box<AudioState> {
        let bufs = |counts: &[u32]| -> Vec<Vec<Vec<f32>>> {
            counts
                .iter()
                .map(|&c| (0..c).map(|_| vec![0.0f32; max_block]).collect())
                .collect()
        };
        let mut in_bufs = bufs(&self.ports.inputs);
        let mut out_bufs = bufs(&self.ports.outputs);
        let ptrs = |b: &mut Vec<Vec<Vec<f32>>>| -> Vec<Vec<*mut f32>> {
            b.iter_mut()
                .map(|port| port.iter_mut().map(|ch| ch.as_mut_ptr()).collect())
                .collect()
        };
        let mut in_ptrs = ptrs(&mut in_bufs);
        let mut out_ptrs = ptrs(&mut out_bufs);
        let audio = |p: &mut Vec<Vec<*mut f32>>| -> Vec<clap_audio_buffer> {
            p.iter_mut()
                .map(|port| clap_audio_buffer {
                    data32: port.as_mut_ptr(),
                    data64: std::ptr::null_mut(),
                    channel_count: port.len() as u32,
                    latency: 0,
                    constant_mask: 0,
                })
                .collect()
        };
        let in_audio = audio(&mut in_ptrs);
        let out_audio = audio(&mut out_ptrs);
        Box::new(AudioState {
            max_block,
            in_bufs,
            out_bufs,
            _in_ptrs: in_ptrs,
            _out_ptrs: out_ptrs,
            in_audio,
            out_audio,
            events: Vec::with_capacity(MAX_CHUNK_EVENTS),
            out_events: Vec::with_capacity(OUT_EVENT_CAPACITY),
            steady: 0,
            processing: false,
            start_failed: false,
        })
    }
}

impl PluginInstance for ClapInstance {
    fn info(&self) -> PluginInfo {
        PluginInfo {
            editor: false,
            name: self.name.clone(),
            vendor: self.vendor.clone(),
            version: self.version.clone(),
            params: self.params.clone(),
            groups: self.groups.clone(),
            values: self.current_values(),
            // A PowerVoice module's text follows the module API's own rules (units, `-inf dB`),
            // which the host applies itself — no control round trip per value.
            param_text: self.module_info.is_none()
                && self.params_ext().is_some_and(|e| e.value_to_text.is_some()),
        }
    }

    fn activate(&self, config: &ActivateConfig) -> Result<ActiveInfo, String> {
        let mut active = lock(&self.active);
        if *active {
            return Err("already active".into());
        }
        let max_block = config.max_block.max(1);
        let state = self.build_audio(max_block as usize);
        let Some(activate) = self.p().activate else {
            return Err(format!("{} can't be activated", self.name));
        };
        // Requests made while being activated are covered by this activation.
        // SAFETY: main thread, inactive plugin.
        let ok = unsafe { activate(self.plugin, config.sample_rate, 1, max_block) };
        self.host
            .state
            .restart_requested
            .store(false, Ordering::Release);
        if !ok {
            return Err(format!("{} refused to activate", self.name));
        }
        // SAFETY: main thread & active: `latency.get` and `tail.get` are allowed.
        let latency = unsafe {
            self.ext
                .latency
                .as_ref()
                .and_then(|e| e.get)
                .map_or(0, |f| f(self.plugin))
        };
        // SAFETY: as above.
        let tail = unsafe {
            self.ext
                .tail
                .as_ref()
                .and_then(|e| e.get)
                .map_or(0, |f| f(self.plugin))
        };
        *lock(&self.audio) = Some(state);
        *active = true;
        Ok(ActiveInfo {
            latency_samples: latency,
            tail_samples: (tail < i32::MAX as u32).then_some(u64::from(tail)),
        })
    }

    fn deactivate(&self) {
        let mut active = lock(&self.active);
        if !*active {
            return;
        }
        // SAFETY: main thread; the audio thread has stopped (and called stop_processing).
        unsafe {
            if let Some(f) = self.p().deactivate {
                f(self.plugin);
            }
        }
        *lock(&self.audio) = None;
        *active = false;
    }

    fn process(&self, chunk: Chunk<'_>, out_events: &mut Vec<WireEvent>) {
        mark_audio_thread();
        let mut guard = lock(&self.audio);
        let n = chunk.input.len().min(chunk.output.len());
        let Some(st) = guard.as_deref_mut().filter(|s| n <= s.max_block) else {
            chunk.output[..n].copy_from_slice(&chunk.input[..n]);
            return;
        };
        let p = self.p();
        if !st.processing && !st.start_failed {
            // SAFETY: audio thread, active plugin.
            if unsafe { p.start_processing.is_none_or(|f| f(self.plugin)) } {
                st.processing = true;
            } else {
                st.start_failed = true;
            }
        }
        let Some(process) = p.process.filter(|_| st.processing) else {
            chunk.output[..n].copy_from_slice(&chunk.input[..n]);
            return;
        };
        st.events.clear();
        for ev in chunk.events {
            if ev.kind == EventKind::RESET {
                // SAFETY: audio thread, active plugin.
                unsafe {
                    if let Some(f) = p.reset {
                        f(self.plugin);
                    }
                }
            } else if ev.kind == EventKind::PARAM_VALUE && st.events.len() < st.events.capacity() {
                let time = chunk.offset(ev) as u32;
                st.events
                    .push(param_event(time, ev.id, self.cookie(ev.id), ev.value));
            }
        }
        // Input: the main port's every channel carries the mono input (upmix); other ports
        // (side chains) get silence.
        for (k, port) in st.in_bufs.iter_mut().enumerate() {
            for ch in port.iter_mut() {
                if k == self.ports.main_in {
                    ch[..n].copy_from_slice(&chunk.input[..n]);
                } else {
                    ch[..n].fill(0.0);
                }
            }
        }
        st.out_events.clear();
        let in_list = clap_input_events {
            ctx: (&raw mut st.events).cast(),
            size: Some(in_size),
            get: Some(in_get),
        };
        let out_list = clap_output_events {
            ctx: (&raw mut st.out_events).cast(),
            try_push: Some(out_try_push),
        };
        let block = clap_process {
            steady_time: st.steady,
            frames_count: n as u32,
            transport: std::ptr::null(),
            audio_inputs: st.in_audio.as_ptr(),
            audio_outputs: st.out_audio.as_mut_ptr(),
            audio_inputs_count: st.in_audio.len() as u32,
            audio_outputs_count: st.out_audio.len() as u32,
            in_events: &in_list,
            out_events: &out_list,
        };
        // SAFETY: audio thread, processing plugin; every buffer holds `max_block` ≥ n frames.
        let status = unsafe { process(self.plugin, &block) };
        st.steady += n as i64;
        if status == CLAP_PROCESS_ERROR {
            chunk.output[..n].copy_from_slice(&chunk.input[..n]);
        } else {
            // Output: the main port's channels averaged (downmix; a mono port is copied).
            let chans = &st.out_bufs[self.ports.main_out];
            if chans.len() == 1 {
                chunk.output[..n].copy_from_slice(&chans[0][..n]);
            } else {
                let scale = 1.0 / chans.len() as f32;
                for (i, o) in chunk.output[..n].iter_mut().enumerate() {
                    let mut acc = chans[0][i];
                    for ch in &chans[1..] {
                        acc += ch[i];
                    }
                    *o = acc * scale;
                }
            }
        }
        for &(time, kind, id, value) in &st.out_events {
            if out_events.len() < out_events.capacity() {
                let pos = chunk.pos + u64::from(time.min(n.saturating_sub(1) as u32));
                out_events.push(WireEvent {
                    pos,
                    kind,
                    id,
                    value,
                });
            }
        }
        if self
            .host
            .state
            .restart_requested
            .swap(false, Ordering::AcqRel)
            && out_events.len() < out_events.capacity()
        {
            out_events.push(WireEvent {
                pos: chunk.pos,
                kind: EventKind::RESTART_REQUEST,
                id: 0,
                value: 0.0,
            });
        }
    }

    fn audio_thread_stopping(&self) {
        if let Some(st) = lock(&self.audio).as_deref_mut()
            && st.processing
        {
            // SAFETY: audio thread (the audio loop calls this before it exits).
            unsafe {
                if let Some(f) = self.p().stop_processing {
                    f(self.plugin);
                }
            }
            st.processing = false;
        }
    }

    fn main_thread_idle(&self, events: &mut MainThreadEvents) {
        let host = &self.host.state;
        if host.callback_requested.swap(false, Ordering::AcqRel) {
            // SAFETY: main thread, as requested by the plugin.
            unsafe {
                if let Some(f) = self.p().on_main_thread {
                    f(self.plugin);
                }
            }
        }
        if host.flush_requested.load(Ordering::Acquire) && !*lock(&self.active) {
            host.flush_requested.store(false, Ordering::Release);
            // T-901: what the plugin reports while inactive (its window's edits) goes to the
            // host as a notification (while active, as `process` output events).
            for (_, kind, id, value) in self.flush(Vec::new()) {
                if kind == EventKind::PARAM_VALUE && self.params.iter().any(|p| p.id.0 == id) {
                    events.params.push(ParamValue {
                        id: ParamId(id),
                        value,
                    });
                }
            }
        }
        if host.state_dirty.swap(false, Ordering::AcqRel) {
            events.state_dirty = true;
        }
        if lock(&self.gui_open).is_some() {
            if host.gui_closed.swap(false, Ordering::AcqRel) {
                events.editor_closed = true;
            }
            let r = host.resize_request.swap(NO_RESIZE, Ordering::AcqRel);
            if r != NO_RESIZE {
                events.editor_resize = Some(((r >> 32) as u32, r as u32));
            }
        }
    }

    fn has_editor(&self) -> bool {
        let Some(supported) = self.gui_ext().and_then(|g| g.is_api_supported) else {
            return false;
        };
        let api = Api::native().clap_name();
        // SAFETY: main thread; a static C string.
        unsafe {
            supported(self.plugin, api.as_ptr(), false)
                || supported(self.plugin, api.as_ptr(), true)
        }
    }

    fn open_editor(&self, host: &mut EditorHost<'_>) -> Result<(u32, u32), String> {
        let gui = self
            .gui_ext()
            .ok_or_else(|| "the plugin has no window of its own".to_owned())?;
        let (Some(supported), Some(create)) = (gui.is_api_supported, gui.create) else {
            return Err(format!("{} has an incomplete GUI extension", self.name));
        };
        let api = host.api()?.clap_name();
        // SAFETY: main thread; a static C string.
        let embedded = unsafe { supported(self.plugin, api.as_ptr(), false) };
        // SAFETY: as above.
        let floating = !embedded && unsafe { supported(self.plugin, api.as_ptr(), true) };
        if !embedded && !floating {
            return Err(format!("{} has no window for this system", self.name));
        }
        let st = &self.host.state;
        st.gui_closed.store(false, Ordering::Release);
        st.resize_request.store(NO_RESIZE, Ordering::Release);
        // SAFETY: as above.
        if !unsafe { create(self.plugin, api.as_ptr(), floating) } {
            return Err(format!("{} couldn't create its window", self.name));
        }
        let shown = if embedded {
            self.attach_embedded(gui, host)
        } else {
            self.show_floating(gui, host)
        };
        match shown {
            Ok(size) => {
                *lock(&self.gui_open) = Some(floating);
                Ok(size)
            }
            Err(e) => {
                // SAFETY: main thread; the GUI created above, destroyed once.
                unsafe {
                    if let Some(destroy) = gui.destroy {
                        destroy(self.plugin);
                    }
                }
                Err(e)
            }
        }
    }

    fn close_editor(&self) {
        if lock(&self.gui_open).take().is_none() {
            return;
        }
        if let Some(gui) = self.gui_ext() {
            // SAFETY: main thread; the open GUI, hidden and destroyed once.
            unsafe {
                if let Some(hide) = gui.hide {
                    hide(self.plugin);
                }
                if let Some(destroy) = gui.destroy {
                    destroy(self.plugin);
                }
            }
        }
        let st = &self.host.state;
        st.gui_closed.store(false, Ordering::Release);
        st.resize_request.store(NO_RESIZE, Ordering::Release);
    }

    fn editor_resized(&self, width: u32, height: u32) -> Option<(u32, u32)> {
        if *lock(&self.gui_open) != Some(false) {
            return None;
        }
        let gui = self.gui_ext()?;
        if !self.gui_resizable(gui) {
            // Snap back to the plugin's size.
            return self.gui_size(gui).filter(|&s| s != (width, height));
        }
        let (mut w, mut h) = (width, height);
        // SAFETY: main thread, an open embedded GUI; writable sizes.
        unsafe {
            if let Some(adjust) = gui.adjust_size {
                adjust(self.plugin, &mut w, &mut h);
            }
            if let Some(set) = gui.set_size {
                set(self.plugin, w, h);
            }
        }
        ((w, h) != (width, height)).then_some((w, h))
    }

    fn set_param(&self, id: ParamId, value: f64) -> Result<(), String> {
        if !self.params.iter().any(|p| p.id == id) {
            return Err(format!("no parameter {}", id.0));
        }
        self.flush(vec![param_event(0, id.0, self.cookie(id.0), value)]);
        Ok(())
    }

    fn save_state(&self) -> Result<Vec<u8>, String> {
        // SAFETY: null or the plugin's vtable.
        let Some(save) = (unsafe { self.ext.state.as_ref() }).and_then(|e| e.save) else {
            return Ok(Vec::new());
        };
        // SAFETY: main thread; the stream lives for the call.
        stream::save(|s| unsafe { save(self.plugin, s) })
            .ok_or_else(|| format!("{} couldn't save its state", self.name))
    }

    fn load_state(&self, data: &[u8]) -> Result<(), String> {
        // SAFETY: null or the plugin's vtable.
        let load = (unsafe { self.ext.state.as_ref() }).and_then(|e| e.load);
        match load {
            _ if data.is_empty() => Ok(()),
            None => Err(format!("{} has no state to load", self.name)),
            // SAFETY: main thread, inactive; the stream lives for the call.
            Some(load) => stream::load(data, |s| unsafe { load(self.plugin, s) })
                .then_some(())
                .ok_or_else(|| format!("{} rejected the state", self.name)),
        }
    }

    fn param_to_text(&self, id: ParamId, value: f64) -> Option<String> {
        self.value_text(id.0, value)
    }

    fn text_to_param(&self, id: ParamId, text: &str) -> Option<f64> {
        let f = self.params_ext()?.text_to_value?;
        let text = CString::new(text).ok()?;
        let mut out = 0.0;
        // SAFETY: main thread; NUL-terminated text; writable `out`.
        (unsafe { f(self.plugin, id.0, text.as_ptr(), &mut out) } && out.is_finite()).then_some(out)
    }
}

/// A `clap_window_t` for `w`.
fn clap_window_of(w: NativeWindow) -> clap_window {
    let handle = match w.api {
        Api::X11 => clap_window_handle {
            x11: w.raw as c_ulong,
        },
        Api::Win32 => clap_window_handle {
            win32: w.raw as usize as *mut c_void,
        },
        Api::Cocoa => clap_window_handle {
            cocoa: w.raw as usize as *mut c_void,
        },
    };
    clap_window {
        api: w.api.clap_name().as_ptr(),
        handle,
    }
}

impl Drop for ClapInstance {
    fn drop(&mut self) {
        // T-901: the GUI goes first, and no timer or descriptor callback may outlive the plugin.
        self.close_editor();
        runloop::clear();
        if *lock(&self.active) {
            self.deactivate();
        }
        // SAFETY: main thread; destroy once, before the library is deinitialised (fields drop
        // after this body: `host`, then `_library`).
        unsafe {
            if let Some(f) = self.p().destroy {
                f(self.plugin);
            }
        }
    }
}

//! One LV2 audio effect as a sandbox [`PluginInstance`] (ADR-008 Amendment 8 §2).
//!
//! - **Discovery** through lilv (its own world, holding only this bundle); **instantiation**
//!   through `lilv_plugin_instantiate` with PowerVoice's [`Host`] features. An instance is bound
//!   to a sample rate and a maximum block: the first one is made at load (48 kHz, 4096 frames),
//!   and an activation with another rate or a bigger block re-instantiates, carrying the
//!   `state:interface` properties and the control values over.
//! - **Control ports** live in one buffer connected at instantiation; the audio thread writes
//!   the inputs as events arrive, `run` writes the outputs, and an atomic mirror lets the main
//!   thread read them (`info`, `save_state`) at any time.
//! - **Sample-accurate control:** a chunk is `run` in segments split at its parameter events'
//!   offsets (the audio ports are re-connected at the segment's offset — `connect_port` is
//!   real-time safe), since a control port has one value per `run`.
//! - **Audio:** every main (non-side-chain) input port gets the mono input; the output is the
//!   mean of the main output ports — CLAP's mono shim (ADR-008 Amendment 3 §2). Side-chain and CV
//!   inputs get silence; atom inputs an empty `atom:Sequence`, atom outputs room for one.
//! - **Latency:** the `lv2:latency` output port, read after a short silent probe `run` at
//!   activation (then `deactivate` + `activate` again, so the stream starts from a fresh state); a
//!   later change asks for a restart once (`EventKind::RESTART_REQUEST`), like CLAP and VST3.
//! - **State:** control values by symbol plus the `state:interface` properties ([`Blob`]).

use std::cell::UnsafeCell;
use std::ffi::{CStr, c_char, c_void};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};

use vox_lv2_abi::*;
use vox_module_api::{ActivateConfig, ParamFlags, ParamGroup, ParamId, ParamInfo, ProcessMode};
use vox_sandbox_ipc::protocol::{ParamValue, PluginInfo};
use vox_sandbox_ipc::{Chunk, EventKind, WireEvent};

use super::host::{Host, PROVIDED_OPTIONS, SUPPORTED_FEATURES};
use super::lilv::{Api, LilvInstanceImpl, Node, Nodes, UiChoice, World};
use super::params::{self, PortKind, RawPort};
use super::state::{Blob, Property};
use super::suil::{self, UiSession};
use super::worker::{self, AudioSide, WorkerSide, WorkerThread};
use crate::backend::{ActiveInfo, MainThreadEvents, PluginInstance};
use crate::gui::{Api as WindowApi, EditorHost};

/// T-901: a [`Lv2Instance::ui_writes`] cell holding a value (its low 32 bits are the `f32`).
const UI_WRITE: u64 = 1 << 32;
/// T-901: the window an LV2 UI opens in, until it asks for its own size (`ui:resize`).
const UI_DEFAULT_SIZE: (u32, u32) = (480, 320);

/// T-901: the plugin's UI suil can show in an X11 window (X11 platforms, suil installed).
fn choose_ui(world: &World, plugin: *const c_void) -> Option<UiChoice> {
    if WindowApi::native() != WindowApi::X11 {
        return None;
    }
    let suil = suil::api().ok()?;
    world.plugin_ui(plugin, uri::UI_X11_UI, suil.ui_supported)
}

/// The sample rate of the instance made at load (re-instantiated when an activation differs).
const DEFAULT_RATE: f64 = 48_000.0;
/// The smallest maximum block announced to a plugin.
const DEFAULT_MAX_BLOCK: u32 = 4096;
/// Atom port buffers (bytes), unless the port asks for more (`rsz:minimumSize`).
const ATOM_BUFFER_BYTES: u32 = 8192;
/// The latency probe's length (frames of silence).
const PROBE_FRAMES: u32 = 256;

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

type P = *const c_void;

/// What the plugin's data says about it.
#[derive(Clone, Debug, Default)]
pub(crate) struct Description {
    pub(crate) uri: String,
    pub(crate) name: String,
    pub(crate) vendor: String,
    pub(crate) version: String,
    pub(crate) description: String,
    pub(crate) url: Option<String>,
    /// `rdf:type` URIs.
    pub(crate) classes: Vec<String>,
    pub(crate) ports: Vec<RawPort>,
    pub(crate) required_features: Vec<String>,
    pub(crate) required_options: Vec<String>,
    /// `state:loadDefaultState` is required or optional.
    pub(crate) load_default_state: bool,
}

impl Description {
    /// `(main inputs, main outputs)`: the audio ports that aren't side chains.
    pub(crate) fn main_audio(&self) -> (Vec<u32>, Vec<u32>) {
        let pick = |kind: PortKind| -> Vec<u32> {
            let all: Vec<&RawPort> = self.ports.iter().filter(|p| p.kind == kind).collect();
            let main: Vec<u32> = all
                .iter()
                .filter(|p| !p.side_chain)
                .map(|p| p.index)
                .collect();
            if main.is_empty() && kind == PortKind::AudioOut {
                all.iter().map(|p| p.index).collect()
            } else {
                main
            }
        };
        (pick(PortKind::AudioIn), pick(PortKind::AudioOut))
    }

    /// Why PowerVoice can't host this plugin, if it can't.
    pub(crate) fn unsupported(&self) -> Option<String> {
        let name = &self.name;
        if let Some(p) = self.ports.iter().find(|p| p.kind == PortKind::Unsupported) {
            return Some(format!(
                "{name}: port “{}” is of a type PowerVoice can't connect",
                p.symbol
            ));
        }
        let supported = |u: &String| SUPPORTED_FEATURES.iter().any(|s| uri_str(s) == u);
        if let Some(f) = self.required_features.iter().find(|f| !supported(f)) {
            return Some(format!(
                "{name} needs the LV2 feature <{f}>, which PowerVoice doesn't provide"
            ));
        }
        let provided = |u: &String| PROVIDED_OPTIONS.iter().any(|s| uri_str(s) == u);
        if let Some(o) = self.required_options.iter().find(|o| !provided(o)) {
            return Some(format!(
                "{name} needs the LV2 option <{o}>, which PowerVoice doesn't provide"
            ));
        }
        let (ins, outs) = self.main_audio();
        if ins.is_empty() || outs.is_empty() {
            return Some(format!(
                "{name} has no audio {} (not an audio effect)",
                if ins.is_empty() { "input" } else { "output" }
            ));
        }
        None
    }
}

fn texts(api: &Api, nodes: Option<Nodes>) -> Vec<String> {
    nodes.map_or_else(Vec::new, |n| {
        n.items()
            .into_iter()
            .filter_map(|x| api.uri_of(x).or_else(|| api.text_of(x)))
            .collect()
    })
}

/// Reads everything PowerVoice needs about `plugin` from its data (no code is loaded).
pub(crate) fn describe(world: &World, plugin: P) -> Description {
    let api = world.api;
    let uri = world.plugin_uri(plugin).unwrap_or_default();
    // SAFETY: a plugin of this world; lilv returns owned nodes (freed by `Node`).
    let owned = |ptr: *mut c_void| Node::own(api, ptr).and_then(|n| api.text_of(n.ptr()));
    // SAFETY: as above.
    let name = unsafe { owned((api.plugin_get_name)(plugin)) }
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| uri.clone());
    // SAFETY: as above.
    let vendor = unsafe { owned((api.plugin_get_author_name)(plugin)) }.unwrap_or_default();
    // SAFETY: as above.
    let url = unsafe { owned((api.plugin_get_author_homepage)(plugin)) }
        .or_else(|| world.plugin_first_text(plugin, uri::DOAP_HOMEPAGE))
        .filter(|s| !s.trim().is_empty());
    let number = |pred: &CStr| {
        world
            .plugin_values(plugin, pred)
            .and_then(|n| n.items().first().and_then(|&x| api.number_of(x)))
    };
    let version = match (number(uri::MINOR_VERSION), number(uri::MICRO_VERSION)) {
        (Some(minor), micro) => format!("{minor}.{}", micro.unwrap_or(0.0)),
        _ => String::new(),
    };
    let description = world
        .plugin_first_text(plugin, uri::RDFS_COMMENT)
        .unwrap_or_default();
    // SAFETY: the plugin's URI node is borrowed from the world.
    let subject = unsafe { (api.plugin_get_uri)(plugin) };
    let classes = texts(api, world.find(subject, uri::RDF_TYPE));
    // SAFETY: an owned collection.
    let required_features = texts(
        api,
        Nodes::own(api, unsafe { (api.plugin_get_required_features)(plugin) }),
    );
    let required_options = texts(
        api,
        world.plugin_values(plugin, uri::OPTIONS_REQUIRED_OPTION),
    );
    let feature = world.uri(uri::STATE_LOAD_DEFAULT_STATE);
    // SAFETY: a plugin of this world and a live node.
    let load_default_state = unsafe { (api.plugin_has_feature)(plugin, feature.ptr()) };
    Description {
        uri,
        name,
        vendor,
        version,
        description,
        url,
        classes,
        ports: read_ports(world, plugin),
        required_features,
        required_options,
        load_default_state,
    }
}

fn read_ports(world: &World, plugin: P) -> Vec<RawPort> {
    let api = world.api;
    let n = |u: &CStr| world.uri(u);
    let (audio, control, cv, atom) = (
        n(uri::AUDIO_PORT),
        n(uri::CONTROL_PORT),
        n(uri::CV_PORT),
        n(uri::ATOM_PORT),
    );
    let (input, output) = (n(uri::INPUT_PORT), n(uri::OUTPUT_PORT));
    let props = [
        n(uri::CONNECTION_OPTIONAL),
        n(uri::IS_SIDE_CHAIN),
        n(uri::TOGGLED),
        n(uri::INTEGER),
        n(uri::ENUMERATION),
        n(uri::PORT_PROPS_LOGARITHMIC),
        n(uri::PORT_PROPS_NOT_AUTOMATIC),
        n(uri::PORT_PROPS_NOT_ON_GUI),
        n(uri::SAMPLE_RATE),
        n(uri::REPORTS_LATENCY),
    ];
    let (designation, unit, group, min_size) = (
        n(uri::DESIGNATION),
        n(uri::UNITS_UNIT),
        n(uri::PORT_GROUPS_GROUP),
        n(uri::RESIZE_PORT_MINIMUM_SIZE),
    );
    // SAFETY: a plugin of this world.
    let count = unsafe { (api.plugin_get_num_ports)(plugin) };
    let mut ports = Vec::with_capacity(count as usize);
    for index in 0..count {
        // SAFETY: `index < count`.
        let port = unsafe { (api.plugin_get_port_by_index)(plugin, index) };
        if port.is_null() {
            continue;
        }
        // SAFETY (the closures): a port of this plugin and live nodes; owned results are freed.
        let is = |c: &Node| unsafe { (api.port_is_a)(plugin, port, c.ptr()) };
        let has = |c: &Node| unsafe { (api.port_has_property)(plugin, port, c.ptr()) };
        let get = |c: &Node| Node::own(api, unsafe { (api.port_get)(plugin, port, c.ptr()) });
        let [
            optional,
            side_chain,
            toggled,
            integer,
            enumeration,
            log,
            not_auto,
            not_gui,
            sr,
            rl,
        ] = props.each_ref().map(has);
        let (is_in, is_out) = (is(&input), is(&output));
        let dir = |a: PortKind, b: PortKind| {
            if is_in {
                a
            } else if is_out {
                b
            } else {
                PortKind::Unsupported
            }
        };
        let mut kind = if is(&audio) {
            dir(PortKind::AudioIn, PortKind::AudioOut)
        } else if is(&control) {
            dir(PortKind::ControlIn, PortKind::ControlOut)
        } else if is(&cv) {
            dir(PortKind::CvIn, PortKind::CvOut)
        } else if is(&atom) {
            dir(PortKind::AtomIn, PortKind::AtomOut)
        } else {
            PortKind::Unsupported
        };
        if kind == PortKind::Unsupported && optional {
            kind = PortKind::Optional;
        }
        let (mut def, mut min, mut max) = (
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        );
        // SAFETY: out-params lilv fills with owned nodes (or null).
        unsafe { (api.port_get_range)(plugin, port, &mut def, &mut min, &mut max) };
        let num = |ptr| Node::own(api, ptr).and_then(|n| api.number_of(n.ptr()));
        let (default, min, max) = (num(def), num(min), num(max));
        let mut scale_points = Vec::new();
        // SAFETY: an owned collection (or null), freed below.
        let sps = unsafe { (api.port_get_scale_points)(plugin, port) };
        if !sps.is_null() {
            // SAFETY: lilv's iteration over a live collection; the points are borrowed.
            unsafe {
                let mut it = (api.scale_points_begin)(sps);
                while !(api.scale_points_is_end)(sps, it) {
                    let sp = (api.scale_points_get)(sps, it);
                    let value = api.number_of((api.scale_point_get_value)(sp));
                    let label = api.text_of((api.scale_point_get_label)(sp));
                    if let (Some(v), Some(l)) = (value, label) {
                        scale_points.push((v, l));
                    }
                    it = (api.scale_points_next)(sps, it);
                }
                (api.scale_points_free)(sps);
            }
        }
        scale_points.sort_by(|a, b| a.0.total_cmp(&b.0));
        let group = get(&group).and_then(|g| {
            [uri::NAME, uri::RDFS_LABEL].into_iter().find_map(|pred| {
                world
                    .find(g.ptr(), pred)
                    .and_then(|ns| ns.items().first().and_then(|&x| api.text_of(x)))
            })
        });
        // SAFETY: the symbol node is borrowed from the plugin.
        let symbol = api
            .text_of(unsafe { (api.port_get_symbol)(plugin, port) })
            .unwrap_or_default();
        // SAFETY: an owned node.
        let name = Node::own(api, unsafe { (api.port_get_name)(plugin, port) })
            .and_then(|n| api.text_of(n.ptr()))
            .unwrap_or_default();
        ports.push(RawPort {
            index,
            symbol,
            name,
            kind,
            side_chain,
            min,
            max,
            default,
            toggled,
            integer,
            enumeration,
            logarithmic: log,
            not_automatic: not_auto,
            not_on_gui: not_gui,
            sample_rate: sr,
            reports_latency: rl,
            unit: get(&unit).and_then(|u| api.uri_of(u.ptr())),
            designation: get(&designation).and_then(|d| api.uri_of(d.ptr())),
            scale_points,
            group,
            minimum_size: get(&min_size)
                .and_then(|m| api.number_of(m.ptr()))
                .map(|v| v.max(0.0) as u32),
        });
    }
    ports
}

/// One `f32` per port index, connected to the plugin's control ports. The main thread writes
/// it only while the plugin can't run (inactive, or before activation); while active only the
/// audio thread (and the plugin's `run`) touch it — every read from elsewhere goes through the
/// atomic mirror.
struct ControlBuf(UnsafeCell<Box<[f32]>>);

// SAFETY: see the type's docs.
unsafe impl Sync for ControlBuf {}

impl ControlBuf {
    fn ptr(&self, index: u32) -> *mut f32 {
        // SAFETY: an index into our own buffer (callers pass port indices < len).
        unsafe { (*self.0.get()).as_mut_ptr().add(index as usize) }
    }

    /// # Safety
    /// The type's access discipline.
    unsafe fn get(&self, index: u32) -> f32 {
        // SAFETY: caller's contract.
        unsafe { *self.ptr(index) }
    }

    /// # Safety
    /// The type's access discipline.
    unsafe fn set(&self, index: u32, v: f32) {
        // SAFETY: caller's contract.
        unsafe { *self.ptr(index) = v }
    }
}

/// The current instance and the host it was made with (main thread).
struct Core {
    api: &'static Api,
    inst: *mut LilvInstanceImpl,
    host: Option<Box<Host>>,
    rate: f64,
    max_block: u32,
    state_iface: *const LV2_State_Interface,
    worker_iface: *const LV2_Worker_Interface,
}

// SAFETY: the instance is only driven from the sandbox's main thread through this struct (the
// audio thread gets its descriptor and handle at activation, see `AudioState`).
unsafe impl Send for Core {}

impl Core {
    fn descriptor(&self) -> (&LV2_Descriptor, LV2_Handle) {
        // SAFETY: a live instance (non-null while the `Lv2Instance` exists); lilv documents the
        // struct's first two fields.
        unsafe { (&*(*self.inst).lv2_descriptor, (*self.inst).lv2_handle) }
    }

    fn host(&self) -> &Host {
        self.host
            .as_deref()
            .expect("an instance always has its host")
    }

    fn free(&mut self) {
        if !self.inst.is_null() {
            // SAFETY: an instance lilv created, freed once (inactive).
            unsafe { (self.api.instance_free)(self.inst) };
            self.inst = std::ptr::null_mut();
        }
        // The host outlives the instance that referenced it.
        self.host = None;
    }
}

impl Drop for Core {
    fn drop(&mut self) {
        self.free();
    }
}

/// The audio thread's working set, built at activation.
struct AudioState {
    max_block: usize,
    desc: *const LV2_Descriptor,
    handle: LV2_Handle,
    /// `(port, main, buffer)`.
    audio_in: Vec<(u32, bool, Vec<f32>)>,
    audio_out: Vec<(u32, bool, Vec<f32>)>,
    /// `(port, input, 8-byte-aligned buffer)`.
    atoms: Vec<(u32, bool, Vec<u64>)>,
    seq: u32,
    chunk_type: u32,
    worker_iface: *const LV2_Worker_Interface,
    worker: Option<AudioSide>,
    /// Offline: the requests are serviced on this thread right after `run`.
    sync_worker: Option<WorkerSide>,
    /// Read-only parameters (control outputs): `(port, last value bits)`.
    outputs: Vec<(u32, u32)>,
    latency: u32,
    restart_sent: bool,
    /// Kept alive: CV buffers the plugin is connected to.
    _cv: Vec<Vec<f32>>,
}

// SAFETY: the raw pointers belong to the active instance, which only this state's owner runs;
// the state moves from the main thread (activation) to the audio thread behind a mutex.
unsafe impl Send for AudioState {}

fn latency_samples(v: f32) -> u32 {
    if v.is_finite() && v > 0.0 {
        v.round().min(u32::MAX as f32) as u32
    } else {
        0
    }
}

/// `lilv_state_restore`'s port callback context (default state).
struct SetCtx<'a> {
    ports: &'a [RawPort],
    controls: &'a ControlBuf,
    values: &'a [AtomicU32],
    float: u32,
}

unsafe extern "C" fn set_port_value(
    symbol: *const c_char,
    user: *mut c_void,
    value: *const c_void,
    size: u32,
    type_: u32,
) {
    if symbol.is_null() || user.is_null() || value.is_null() {
        return;
    }
    // SAFETY: `user` is the `SetCtx` of the running call; `symbol` is NUL-terminated.
    let (ctx, symbol) = unsafe { (&*(user as *const SetCtx<'_>), CStr::from_ptr(symbol)) };
    if type_ != ctx.float || size != 4 {
        return;
    }
    let Some(p) = ctx
        .ports
        .iter()
        .find(|p| p.kind == PortKind::ControlIn && p.symbol.as_bytes() == symbol.to_bytes())
    else {
        return;
    };
    // SAFETY: 4 bytes of float (possibly unaligned); the plugin is inactive (load time).
    unsafe {
        let v = std::ptr::read_unaligned(value as *const f32);
        ctx.controls.set(p.index, v);
        ctx.values[p.index as usize].store(v.to_bits(), Ordering::Release);
    }
}

/// `state:interface` save's store context.
struct StoreCtx<'a> {
    host: &'a Host,
    props: Vec<Property>,
}

unsafe extern "C" fn store_cb(
    handle: *mut c_void,
    key: u32,
    value: *const c_void,
    size: usize,
    type_: u32,
    flags: u32,
) -> u32 {
    if handle.is_null() || (value.is_null() && size > 0) {
        return LV2_STATE_ERR_UNKNOWN;
    }
    if flags & LV2_STATE_IS_POD == 0 {
        // Not plain data (pointers, handles): meaningless in another process.
        return 3; // LV2_STATE_ERR_BAD_FLAGS
    }
    // SAFETY: `handle` is the `StoreCtx` of the running save.
    let ctx = unsafe { &mut *(handle as *mut StoreCtx<'_>) };
    let (Some(key), Some(type_)) = (ctx.host.urid.unmap(key), ctx.host.urid.unmap(type_)) else {
        return LV2_STATE_ERR_UNKNOWN;
    };
    let value = if size == 0 {
        Vec::new()
    } else {
        // SAFETY: `size` bytes at `value`, valid during the call.
        unsafe { std::slice::from_raw_parts(value.cast::<u8>(), size) }.to_vec()
    };
    ctx.props.retain(|p| p.key != key);
    ctx.props.push(Property {
        key,
        type_,
        flags,
        value,
    });
    LV2_STATE_SUCCESS
}

/// One property for `retrieve`: `(key URID, type URID, flags, size, 8-byte-aligned bytes)`.
type RetrieveItem = (u32, u32, u32, usize, Box<[u64]>);

/// `state:interface` restore's retrieve context.
struct RetrieveCtx {
    items: Vec<RetrieveItem>,
}

unsafe extern "C" fn retrieve_cb(
    handle: *mut c_void,
    key: u32,
    size: *mut usize,
    type_: *mut u32,
    flags: *mut u32,
) -> *const c_void {
    if handle.is_null() {
        return std::ptr::null();
    }
    // SAFETY: `handle` is the `RetrieveCtx` of the running restore.
    let ctx = unsafe { &*(handle as *const RetrieveCtx) };
    let Some((_, t, f, len, bytes)) = ctx.items.iter().find(|i| i.0 == key) else {
        return std::ptr::null();
    };
    // SAFETY: the plugin's out-params (each may be null).
    unsafe {
        if !size.is_null() {
            *size = *len;
        }
        if !type_.is_null() {
            *type_ = *t;
        }
        if !flags.is_null() {
            *flags = *f;
        }
    }
    bytes.as_ptr().cast()
}

/// A loaded LV2 plugin (ADR-008 Amendment 8).
pub struct Lv2Instance {
    api: &'static Api,
    plugin: P,
    name: String,
    vendor: String,
    version: String,
    ports: Vec<RawPort>,
    params: Vec<ParamInfo>,
    groups: Vec<ParamGroup>,
    has_text: bool,
    main_in: Vec<u32>,
    main_out: Vec<u32>,
    latency_port: Option<u32>,
    free_wheel_port: Option<u32>,
    atom_bytes: u32,
    controls: ControlBuf,
    /// Per port: the last known control value (`f32` bits).
    values: Box<[AtomicU32]>,
    core: Mutex<Core>,
    active: Mutex<bool>,
    audio: Mutex<Option<Box<AudioState>>>,
    worker: Mutex<Option<WorkerThread>>,
    /// T-901: the plugin's UI suil can show (`None`: no window).
    ui: Option<UiChoice>,
    /// T-901: the open UI (main thread).
    ui_session: Mutex<Option<UiSession>>,
    /// T-901: per port, a value the UI wrote for the audio thread ([`UI_WRITE`] | `f32` bits;
    /// 0: none), and whether any cell is set.
    ui_writes: Box<[AtomicU64]>,
    ui_any: AtomicBool,
    /// Dropped last: the plugin and everything lilv returned borrow from it.
    world: World,
}

// SAFETY: the LV2 threading classes are kept by the sandbox: instantiation-class calls
// (instantiate, activate, deactivate, state) come from the main thread only, `run` and
// `connect_port` from the audio thread only (under `audio`), `work` from the worker thread; the
// lilv world and plugin are only used on the main thread.
unsafe impl Send for Lv2Instance {}
// SAFETY: see `Send`.
unsafe impl Sync for Lv2Instance {}

impl Lv2Instance {
    /// Loads plugin `uri` from the bundle directory `bundle` (main thread).
    pub fn load(bundle: &Path, uri: &str) -> Result<Self, String> {
        let api = super::lilv::api()?;
        let world = World::new(api)?;
        world.load_bundle(bundle)?;
        let plugin = world
            .plugin(uri)
            .ok_or_else(|| format!("{} has no LV2 plugin <{uri}>", bundle.display()))?;
        // SAFETY: a plugin of this world.
        if !unsafe { (api.plugin_verify)(plugin) } {
            return Err(format!(
                "<{uri}> in {} is not a valid LV2 plugin",
                bundle.display()
            ));
        }
        let d = describe(&world, plugin);
        if let Some(why) = d.unsupported() {
            return Err(why);
        }
        let (main_in, main_out) = d.main_audio();
        let port_count = d.ports.iter().map(|p| p.index + 1).max().unwrap_or(0) as usize;
        let mut init = vec![0.0f32; port_count];
        for p in &d.ports {
            if p.kind == PortKind::ControlIn {
                let v = p.default.or(p.min).unwrap_or(0.0);
                let scale = if p.sample_rate { DEFAULT_RATE } else { 1.0 };
                init[p.index as usize] = (v * scale) as f32;
            }
        }
        let values: Box<[AtomicU32]> = init.iter().map(|v| AtomicU32::new(v.to_bits())).collect();
        let (params, groups) = params::map(&d.ports, DEFAULT_RATE);
        let has_text = d
            .ports
            .iter()
            .any(|p| p.is_param() && !p.scale_points.is_empty());
        let atom_bytes = d
            .ports
            .iter()
            .filter(|p| matches!(p.kind, PortKind::AtomIn | PortKind::AtomOut))
            .filter_map(|p| p.minimum_size)
            .fold(ATOM_BUFFER_BYTES, u32::max)
            .next_multiple_of(8);
        let me = Self {
            api,
            plugin,
            name: d.name.clone(),
            vendor: d.vendor.clone(),
            version: d.version.clone(),
            latency_port: d.ports.iter().find(|p| p.is_latency()).map(|p| p.index),
            free_wheel_port: d
                .ports
                .iter()
                .find(|p| p.is_free_wheeling())
                .map(|p| p.index),
            ports: d.ports,
            params,
            groups,
            has_text,
            main_in,
            main_out,
            atom_bytes,
            controls: ControlBuf(UnsafeCell::new(init.into_boxed_slice())),
            values,
            core: Mutex::new(Core {
                api,
                inst: std::ptr::null_mut(),
                host: None,
                rate: DEFAULT_RATE,
                max_block: DEFAULT_MAX_BLOCK,
                state_iface: std::ptr::null(),
                worker_iface: std::ptr::null(),
            }),
            active: Mutex::new(false),
            audio: Mutex::new(None),
            worker: Mutex::new(None),
            ui: choose_ui(&world, plugin),
            ui_session: Mutex::new(None),
            ui_writes: (0..port_count).map(|_| AtomicU64::new(0)).collect(),
            ui_any: AtomicBool::new(false),
            world,
        };
        {
            let mut core = lock(&me.core);
            me.instantiate(&mut core, DEFAULT_RATE, DEFAULT_MAX_BLOCK)?;
            if d.load_default_state {
                me.load_default_state(&core);
            }
        }
        Ok(me)
    }

    /// Parameter count (T-804 richer scan data).
    pub(crate) fn param_count(&self) -> u32 {
        self.params.len() as u32
    }

    /// `(main input ports, main output ports)` (T-804 richer scan data).
    pub(crate) fn main_ports(&self) -> (u32, u32) {
        (self.main_in.len() as u32, self.main_out.len() as u32)
    }

    fn port(&self, index: u32) -> Option<&RawPort> {
        self.ports.iter().find(|p| p.index == index)
    }

    /// A new instance in `core` (none may exist) at `rate`, run with at most `max_block`.
    fn instantiate(&self, core: &mut Core, rate: f64, max_block: u32) -> Result<(), String> {
        let host = Host::new(rate, max_block, self.atom_bytes);
        // SAFETY: a plugin of our world; the features live in `host`, which the core keeps for
        // the instance's lifetime.
        let inst = unsafe { (self.api.plugin_instantiate)(self.plugin, rate, host.features()) };
        if inst.is_null() {
            return Err(format!("{} couldn't be instantiated", self.name));
        }
        core.inst = inst;
        core.host = Some(host);
        core.rate = rate;
        core.max_block = max_block;
        // SAFETY: the fresh instance; lilv documents the struct's first two fields, and the
        // descriptor lives as long as the plugin's library.
        let (desc, handle) = unsafe { (&*(*inst).lv2_descriptor, (*inst).lv2_handle) };
        let (Some(connect), Some(_)) = (desc.connect_port, desc.run) else {
            core.free();
            return Err(format!("{} has an incomplete LV2 descriptor", self.name));
        };
        let ext = |u: &CStr| {
            // SAFETY: the descriptor's discovery-class function with a NUL-terminated URI.
            desc.extension_data
                .map_or(std::ptr::null(), |f| unsafe { f(u.as_ptr()) })
        };
        core.state_iface = ext(uri::STATE_INTERFACE).cast();
        core.worker_iface = ext(uri::WORKER_INTERFACE).cast();
        for p in &self.ports {
            let data: *mut c_void = match p.kind {
                PortKind::ControlIn | PortKind::ControlOut => self.controls.ptr(p.index).cast(),
                _ => std::ptr::null_mut(),
            };
            // SAFETY: a fresh instance; control buffers outlive it; other ports are connected
            // at activation (null until then).
            unsafe { connect(handle, p.index, data) };
        }
        Ok(())
    }

    /// Re-instantiates at `rate` / `max_block`, carrying the plugin's state over.
    fn reinstantiate(&self, core: &mut Core, rate: f64, max_block: u32) -> Result<(), String> {
        let props = self.save_properties(core).unwrap_or_default();
        core.free();
        self.instantiate(core, rate, max_block)?;
        if !props.is_empty() {
            self.restore_properties(core, &props);
        }
        Ok(())
    }

    /// `state:loadDefaultState`: the plugin's own default state from its data.
    fn load_default_state(&self, core: &Core) {
        let host = core.host();
        // SAFETY: our world and plugin; the map lives in `host`.
        let state = unsafe {
            (self.api.state_new_from_world)(
                self.world.ptr,
                host.map(),
                (self.api.plugin_get_uri)(self.plugin),
            )
        };
        if state.is_null() {
            return;
        }
        let ctx = SetCtx {
            ports: &self.ports,
            controls: &self.controls,
            values: &self.values,
            float: host.u.atom_float,
        };
        // SAFETY: an inactive instance; `ctx` lives for the call; the state is freed once.
        unsafe {
            (self.api.state_restore)(
                state,
                core.inst,
                Some(set_port_value),
                (&raw const ctx).cast_mut().cast(),
                0,
                host.state_features(),
            );
            (self.api.state_free)(state);
        }
    }

    fn save_properties(&self, core: &Core) -> Result<Vec<Property>, String> {
        // SAFETY: null or the plugin's interface (valid while the library is loaded).
        let Some(save) = (unsafe { core.state_iface.as_ref() }).and_then(|i| i.save) else {
            return Ok(Vec::new());
        };
        let host = core.host();
        let (_, handle) = core.descriptor();
        let mut ctx = StoreCtx {
            host,
            props: Vec::new(),
        };
        // SAFETY: `save` may run concurrently with `run` (LV2 state); the context lives for the
        // call.
        let status = unsafe {
            save(
                handle,
                store_cb,
                (&raw mut ctx).cast(),
                LV2_STATE_IS_POD | LV2_STATE_IS_PORTABLE,
                host.state_features(),
            )
        };
        if status != LV2_STATE_SUCCESS {
            return Err(format!("{} couldn't save its state", self.name));
        }
        Ok(ctx.props)
    }

    fn restore_properties(&self, core: &Core, props: &[Property]) {
        // SAFETY: null or the plugin's interface.
        let Some(restore) = (unsafe { core.state_iface.as_ref() }).and_then(|i| i.restore) else {
            return;
        };
        let host = core.host();
        let items = props
            .iter()
            .map(|p| {
                let mut aligned = vec![0u64; p.value.len().div_ceil(8)].into_boxed_slice();
                // SAFETY: `aligned` has at least `len` bytes; plain byte copy.
                unsafe {
                    std::ptr::copy_nonoverlapping(
                        p.value.as_ptr(),
                        aligned.as_mut_ptr().cast::<u8>(),
                        p.value.len(),
                    );
                }
                (
                    host.urid.map_str(&p.key),
                    host.urid.map_str(&p.type_),
                    p.flags,
                    p.value.len(),
                    aligned,
                )
            })
            .collect();
        let ctx = RetrieveCtx { items };
        let (_, handle) = core.descriptor();
        // SAFETY: an inactive instance (main thread); the context lives for the call.
        let status = unsafe {
            restore(
                handle,
                retrieve_cb,
                (&raw const ctx).cast_mut().cast(),
                LV2_STATE_IS_POD | LV2_STATE_IS_PORTABLE,
                host.state_features(),
            )
        };
        if status != LV2_STATE_SUCCESS {
            eprintln!(
                "powervoice-sandbox: {} [warning] state restore returned {status}",
                self.name
            );
        }
    }

    /// Sets control input `index` (inactive, or on the audio thread) and its mirror.
    fn set_control(&self, index: u32, v: f32) {
        // SAFETY: the `ControlBuf` discipline — callers are the main thread while inactive or
        // the audio thread while active.
        unsafe { self.controls.set(index, v) };
        if let Some(a) = self.values.get(index as usize) {
            a.store(v.to_bits(), Ordering::Release);
        }
    }

    fn mirror(&self, index: u32) -> f32 {
        self.values
            .get(index as usize)
            .map_or(0.0, |a| f32::from_bits(a.load(Ordering::Acquire)))
    }

    fn build_audio(&self, max_block: usize, core: &Core) -> Box<AudioState> {
        let (desc, handle) = core.descriptor();
        let host = core.host();
        let mut st = AudioState {
            max_block,
            desc,
            handle,
            audio_in: Vec::new(),
            audio_out: Vec::new(),
            atoms: Vec::new(),
            seq: host.u.atom_sequence,
            chunk_type: host.u.atom_chunk,
            worker_iface: core.worker_iface,
            worker: None,
            sync_worker: None,
            outputs: Vec::new(),
            latency: 0,
            restart_sent: false,
            _cv: Vec::new(),
        };
        let words = (self.atom_bytes as usize).div_ceil(8);
        for p in &self.ports {
            match p.kind {
                PortKind::AudioIn => {
                    st.audio_in.push((
                        p.index,
                        self.main_in.contains(&p.index),
                        vec![0.0; max_block],
                    ));
                }
                PortKind::AudioOut => {
                    st.audio_out.push((
                        p.index,
                        self.main_out.contains(&p.index),
                        vec![0.0; max_block],
                    ));
                }
                PortKind::AtomIn => st.atoms.push((p.index, true, vec![0; words])),
                PortKind::AtomOut => st.atoms.push((p.index, false, vec![0; words])),
                PortKind::CvIn | PortKind::CvOut => st._cv.push(vec![0.0; max_block]),
                _ => {}
            }
        }
        st.outputs = self
            .params
            .iter()
            .filter(|p| p.flags.contains(ParamFlags::READ_ONLY))
            .filter(|p| {
                self.port(p.id.0)
                    .is_some_and(|r| r.kind == PortKind::ControlOut)
            })
            .map(|p| (p.id.0, self.mirror(p.id.0).to_bits()))
            .collect();
        Box::new(st)
    }

    /// Connects every non-control port of an activation (audio at offset 0).
    ///
    /// # Safety
    /// The instance is inactive or being activated (main thread); `st`'s buffers outlive it.
    unsafe fn connect_all(&self, st: &mut AudioState) {
        // SAFETY: the activation's descriptor.
        let desc = unsafe { &*st.desc };
        let Some(connect) = desc.connect_port else {
            return;
        };
        let mut cv = st._cv.iter_mut();
        for p in &self.ports {
            let data: *mut c_void = match p.kind {
                PortKind::AudioIn => st
                    .audio_in
                    .iter_mut()
                    .find(|a| a.0 == p.index)
                    .map_or(std::ptr::null_mut(), |a| a.2.as_mut_ptr().cast()),
                PortKind::AudioOut => st
                    .audio_out
                    .iter_mut()
                    .find(|a| a.0 == p.index)
                    .map_or(std::ptr::null_mut(), |a| a.2.as_mut_ptr().cast()),
                PortKind::AtomIn | PortKind::AtomOut => st
                    .atoms
                    .iter_mut()
                    .find(|a| a.0 == p.index)
                    .map_or(std::ptr::null_mut(), |a| a.2.as_mut_ptr().cast()),
                PortKind::CvIn | PortKind::CvOut => cv
                    .next()
                    .map_or(std::ptr::null_mut(), |b| b.as_mut_ptr().cast()),
                PortKind::Optional => std::ptr::null_mut(),
                // Control ports stay connected to the control buffer.
                _ => continue,
            };
            // SAFETY: caller's contract.
            unsafe { connect(st.handle, p.index, data) };
        }
    }

    /// Runs `len` frames starting at `offset` of the chunk buffers (audio thread, or the main
    /// thread's latency probe). No allocation.
    ///
    /// # Safety
    /// The instance is active and only the calling thread runs it; `offset + len <= max_block`.
    unsafe fn run_segment(st: &mut AudioState, offset: usize, len: usize) {
        // SAFETY: the active instance's descriptor (checked complete at instantiation).
        let desc = unsafe { &*st.desc };
        let (Some(connect), Some(run)) = (desc.connect_port, desc.run) else {
            return;
        };
        let h = st.handle;
        // SAFETY: caller's contract; buffers hold `max_block` frames.
        unsafe {
            for (port, _, buf) in st.audio_in.iter_mut().chain(st.audio_out.iter_mut()) {
                connect(h, *port, buf.as_mut_ptr().add(offset).cast());
            }
            for (_, input, buf) in &mut st.atoms {
                let seq = buf.as_mut_ptr().cast::<LV2_Atom_Sequence>();
                if *input {
                    (*seq).atom = LV2_Atom {
                        size: std::mem::size_of::<LV2_Atom_Sequence_Body>() as u32,
                        type_: st.seq,
                    };
                    (*seq).body = LV2_Atom_Sequence_Body::default();
                } else {
                    (*seq).atom = LV2_Atom {
                        size: (buf.len() * 8 - std::mem::size_of::<LV2_Atom>()) as u32,
                        type_: st.chunk_type,
                    };
                }
            }
            run(h, len as u32);
            if let Some(iface) = st.worker_iface.as_ref() {
                if let Some(w) = st.sync_worker.as_mut() {
                    w.service(iface, h);
                }
                if let Some(a) = st.worker.as_mut() {
                    a.deliver(iface, h);
                }
            }
        }
    }
}

impl PluginInstance for Lv2Instance {
    fn info(&self) -> PluginInfo {
        PluginInfo {
            editor: false,
            name: self.name.clone(),
            vendor: self.vendor.clone(),
            version: self.version.clone(),
            params: self.params.clone(),
            groups: self.groups.clone(),
            values: self
                .params
                .iter()
                .map(|p| {
                    let v = f64::from(self.mirror(p.id.0));
                    ParamValue {
                        id: p.id,
                        value: if v.is_finite() {
                            v.clamp(p.min, p.max)
                        } else {
                            p.default
                        },
                    }
                })
                .collect(),
            param_text: self.has_text,
        }
    }

    fn activate(&self, config: &ActivateConfig) -> Result<ActiveInfo, String> {
        let mut active = lock(&self.active);
        if *active {
            return Err("already active".into());
        }
        let mut core = lock(&self.core);
        let max_block = config.max_block.max(1);
        if core.rate.to_bits() != config.sample_rate.to_bits() || core.max_block < max_block {
            self.reinstantiate(
                &mut core,
                config.sample_rate,
                max_block.max(DEFAULT_MAX_BLOCK),
            )?;
        }
        let offline = config.mode == ProcessMode::Offline;
        if let Some(fw) = self.free_wheel_port {
            self.set_control(fw, if offline { 1.0 } else { 0.0 });
        }
        let mut st = self.build_audio(max_block as usize, &core);
        // SAFETY: an inactive instance, being activated here on the main thread.
        unsafe { self.connect_all(&mut st) };
        let mut worker_side = None;
        if !core.worker_iface.is_null() {
            let (tx, audio_side, side) = worker::rings();
            // SAFETY: inactive: nothing runs the plugin.
            unsafe { core.host().worker.set(Some(tx)) };
            st.worker = Some(audio_side);
            worker_side = Some(side);
        }
        let (desc, handle) = core.descriptor();
        let (act, deact) = (desc.activate, desc.deactivate);
        // SAFETY: instantiation-class calls on the main thread; every port is connected.
        unsafe {
            if let Some(f) = act {
                f(handle);
            }
        }
        let latency = match self.latency_port {
            Some(lp) => {
                // Latency is only valid after a `run`: probe with silence, then start over.
                st.sync_worker = worker_side.take();
                let frames = PROBE_FRAMES.min(max_block) as usize;
                // SAFETY: the activating thread is the only one running the plugin.
                unsafe { Self::run_segment(&mut st, 0, frames) };
                worker_side = st.sync_worker.take();
                // SAFETY: the plugin is not running (ControlBuf discipline).
                let l = latency_samples(unsafe { self.controls.get(lp) });
                // SAFETY: instantiation-class calls on the main thread.
                unsafe {
                    if let Some(f) = deact {
                        f(handle);
                    }
                    if let Some(f) = act {
                        f(handle);
                    }
                }
                l
            }
            None => 0,
        };
        st.latency = latency;
        if offline {
            st.sync_worker = worker_side.take();
        }
        if let Some(side) = worker_side {
            // SAFETY: the interface and handle outlive the thread (joined in `deactivate`).
            match unsafe { WorkerThread::spawn(side, core.worker_iface, handle) } {
                Ok(t) => *lock(&self.worker) = Some(t),
                Err(e) => {
                    // SAFETY: undo the activation (main thread).
                    unsafe {
                        if let Some(f) = deact {
                            f(handle);
                        }
                        core.host().worker.set(None);
                    }
                    return Err(e);
                }
            }
        }
        *lock(&self.audio) = Some(st);
        *active = true;
        Ok(ActiveInfo {
            latency_samples: latency,
            // LV2 has no tail report.
            tail_samples: Some(0),
        })
    }

    fn deactivate(&self) {
        let mut active = lock(&self.active);
        if !*active {
            return;
        }
        // The worker stops before the plugin deactivates.
        *lock(&self.worker) = None;
        let core = lock(&self.core);
        let (desc, handle) = core.descriptor();
        // SAFETY: main thread; the audio thread has stopped serving (the sandbox's contract).
        unsafe {
            if let Some(f) = desc.deactivate {
                f(handle);
            }
            core.host().worker.set(None);
        }
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
        if n == 0 {
            return;
        }
        for (_, main, buf) in &mut st.audio_in {
            if *main {
                buf[..n].copy_from_slice(&chunk.input[..n]);
            } else {
                buf[..n].fill(0.0);
            }
        }
        // T-901: values the plugin's UI wrote apply from this chunk's first sample, and reach the
        // host's mirror like any plugin-originated change.
        if self.ui_any.swap(false, Ordering::AcqRel) {
            for p in &self.ports {
                if p.kind != PortKind::ControlIn {
                    continue;
                }
                let Some(cell) = self.ui_writes.get(p.index as usize) else {
                    continue;
                };
                let w = cell.swap(0, Ordering::AcqRel);
                if w & UI_WRITE != 0 {
                    let v = f32::from_bits(w as u32);
                    self.set_control(p.index, v);
                    if out_events.len() < out_events.capacity() {
                        out_events.push(WireEvent {
                            pos: chunk.pos,
                            kind: EventKind::PARAM_VALUE,
                            id: p.index,
                            value: f64::from(v),
                        });
                    }
                }
            }
        }
        // Segments split at the parameter events' offsets: a control port has one value per
        // `run`.
        let events = chunk.events;
        let (mut i, mut seg) = (0usize, 0usize);
        while seg < n {
            while let Some(ev) = events.get(i) {
                if ev.kind == EventKind::PARAM_VALUE {
                    if chunk.offset(ev) > seg {
                        break;
                    }
                    if self
                        .port(ev.id)
                        .is_some_and(|p| p.kind == PortKind::ControlIn)
                    {
                        self.set_control(ev.id, ev.value as f32);
                    }
                }
                i += 1;
            }
            let end = events.get(i).map_or(n, |ev| chunk.offset(ev).min(n));
            // SAFETY: the audio thread runs the active instance; `end <= n <= max_block`.
            unsafe { Self::run_segment(st, seg, end - seg) };
            seg = end;
        }
        // Output: the main output ports averaged (downmix; a single port is copied).
        let mut count = 0usize;
        for (_, main, buf) in &st.audio_out {
            if !*main {
                continue;
            }
            if count == 0 {
                chunk.output[..n].copy_from_slice(&buf[..n]);
            } else {
                for (o, s) in chunk.output[..n].iter_mut().zip(&buf[..n]) {
                    *o += *s;
                }
            }
            count += 1;
        }
        if count > 1 {
            let scale = 1.0 / count as f32;
            for o in &mut chunk.output[..n] {
                *o *= scale;
            }
        }
        for (port, prev) in &mut st.outputs {
            // SAFETY: the audio thread owns the control buffer while active.
            let v = unsafe { self.controls.get(*port) };
            if v.to_bits() != *prev && out_events.len() < out_events.capacity() {
                *prev = v.to_bits();
                if let Some(a) = self.values.get(*port as usize) {
                    a.store(v.to_bits(), Ordering::Release);
                }
                out_events.push(WireEvent {
                    pos: chunk.pos,
                    kind: EventKind::PARAM_VALUE,
                    id: *port,
                    value: f64::from(v),
                });
            }
        }
        if let Some(lp) = self.latency_port {
            // SAFETY: as above.
            let l = latency_samples(unsafe { self.controls.get(lp) });
            if let Some(a) = self.values.get(lp as usize) {
                a.store((l as f32).to_bits(), Ordering::Release);
            }
            if l != st.latency && !st.restart_sent && out_events.len() < out_events.capacity() {
                st.restart_sent = true;
                out_events.push(WireEvent {
                    pos: chunk.pos,
                    kind: EventKind::RESTART_REQUEST,
                    id: 0,
                    value: 0.0,
                });
            }
        }
    }

    fn main_thread_idle(&self, events: &mut MainThreadEvents) {
        let mut guard = lock(&self.ui_session);
        let Some(ui) = guard.as_mut() else {
            return;
        };
        if !ui.idle() {
            events.editor_closed = true;
        }
        let active = *lock(&self.active);
        for (port, v) in ui.ctl.take_writes() {
            if !self
                .port(port)
                .is_some_and(|p| p.kind == PortKind::ControlIn && p.is_param())
            {
                continue;
            }
            if active {
                if let Some(cell) = self.ui_writes.get(port as usize) {
                    cell.store(UI_WRITE | u64::from(v.to_bits()), Ordering::Release);
                    self.ui_any.store(true, Ordering::Release);
                }
            } else {
                // Inactive: nobody runs the plugin; the host gets the value as a notification.
                self.set_control(port, v);
                events.params.push(ParamValue {
                    id: ParamId(port),
                    value: f64::from(v),
                });
            }
        }
        ui.update(|port| self.mirror(port));
        if let Some(size) = ui.ctl.take_resize() {
            events.editor_resize = Some(size);
        }
    }

    fn has_editor(&self) -> bool {
        self.ui.is_some()
    }

    fn open_editor(&self, host: &mut EditorHost<'_>) -> Result<(u32, u32), String> {
        let choice = self
            .ui
            .as_ref()
            .ok_or_else(|| "the plugin has no window of its own".to_owned())?;
        if host.api()? != WindowApi::X11 {
            return Err(format!("{} has no window for this system", self.name));
        }
        let suil = suil::api()?;
        let plugin_uri = self.world.plugin_uri(self.plugin).unwrap_or_default();
        let (w, h) = UI_DEFAULT_SIZE;
        let window = host.create_window(w, h, true)?;
        let symbols = self
            .ports
            .iter()
            .map(|p| (p.symbol.clone(), p.index))
            .collect();
        let controls = self
            .ports
            .iter()
            .filter(|p| matches!(p.kind, PortKind::ControlIn | PortKind::ControlOut))
            .map(|p| p.index)
            .collect();
        let mut ui = UiSession::open(suil, choice, &plugin_uri, window.raw, symbols, controls)
            .map_err(|e| format!("{}: {e}", self.name))?;
        ui.update(|port| self.mirror(port));
        let size = ui.ctl.take_resize().unwrap_or((w, h));
        host.resize_window(size.0, size.1);
        *lock(&self.ui_session) = Some(ui);
        Ok(size)
    }

    fn close_editor(&self) {
        drop(lock(&self.ui_session).take());
    }

    fn set_param(&self, id: ParamId, value: f64) -> Result<(), String> {
        let Some(p) = self.params.iter().find(|p| p.id == id) else {
            return Err(format!("no parameter {}", id.0));
        };
        if p.flags.contains(ParamFlags::READ_ONLY) {
            return Err(format!("parameter {} is read-only", id.0));
        }
        self.set_control(id.0, value as f32);
        Ok(())
    }

    fn save_state(&self) -> Result<Vec<u8>, String> {
        let core = lock(&self.core);
        let blob = Blob {
            controls: self
                .ports
                .iter()
                .filter(|p| p.kind == PortKind::ControlIn && !p.is_free_wheeling())
                .map(|p| (p.symbol.clone(), self.mirror(p.index)))
                .collect(),
            properties: self.save_properties(&core)?,
        };
        Ok(blob.encode())
    }

    fn load_state(&self, data: &[u8]) -> Result<(), String> {
        if data.is_empty() {
            return Ok(());
        }
        let blob = Blob::decode(data)?;
        for (symbol, v) in &blob.controls {
            if let Some(p) = self.ports.iter().find(|p| {
                p.kind == PortKind::ControlIn && !p.is_free_wheeling() && &p.symbol == symbol
            }) {
                self.set_control(p.index, *v);
            }
        }
        if !blob.properties.is_empty() {
            let core = lock(&self.core);
            self.restore_properties(&core, &blob.properties);
        }
        Ok(())
    }

    fn param_to_text(&self, id: ParamId, value: f64) -> Option<String> {
        self.port(id.0)?.label_of(value).map(str::to_owned)
    }

    fn text_to_param(&self, id: ParamId, text: &str) -> Option<f64> {
        let t = text.trim();
        self.port(id.0)?
            .scale_points
            .iter()
            .find(|(_, l)| l.eq_ignore_ascii_case(t))
            .map(|(v, _)| *v)
    }
}

impl Drop for Lv2Instance {
    fn drop(&mut self) {
        // T-901: the UI goes before the plugin it controls.
        self.close_editor();
        self.deactivate();
        // The instance is freed (main thread) before the world: `core` drops before `world`.
        lock(&self.core).free();
    }
}

//! The in-repo **test CLAP plugin** (T-803): a `cdylib` exporting `clap_entry`, used by the
//! sandbox's CLAP backend tests instead of installed plugins. Its factory offers:
//!
//! | Id | Ports | Parameters | Latency |
//! |---|---|---|---|
//! | [`ID_GAIN`] | mono in/out | `gain_db` ([`PARAM_GAIN`]) | [`LATENCY_SAMPLES`] |
//! | [`ID_GAIN_STEREO`] | **stereo only** in/out (the host's mono shim) | `gain_db` | [`LATENCY_SAMPLES`] |
//! | [`ID_LATENCY`] | mono in/out | `gain_db`, `latency` ([`PARAM_LATENCY`]) | the `latency` value |
//!
//! Every variant runs the built-in Gain (`vox_modules::Gain`, bit-exact with the in-process
//! module) per channel, then a delay line of its latency. Changing `latency` while active calls
//! `clap_host::request_restart`; the new latency applies at the next activation.
//!
//! - **State:** [`STATE_MAGIC`], version `u32` = 1, `gain_db` `f64`, `latency` `u32`
//!   (little-endian).
//! - **Text:** `gain_db` formats as `"{:.2} dB"`, `latency` as `"{} smp"` — different from the
//!   module API's own text rules, so tests can tell whose text the host shows.
//! - A copy whose file name contains [`CRASH_ON_SCAN_MARKER`] aborts in `clap_entry.init`
//!   (the "crashing bundle during scan" test); [`HANG_ON_SCAN_MARKER`] never returns from it.
//! - **GUI** (T-901, [`gui`]): every variant has a trivial embedded X11 editor
//!   ([`GUI_WIDTH`]×[`GUI_HEIGHT`]) whose timer makes one edit — `gain_db` =
//!   [`GUI_EDIT_GAIN_DB`] and the GUI-only [`GUI_MARKER`] in the state (version 2). A copy named
//!   with [`GUI_CRASH_MARKER`] crashes in that timer, [`GUI_HANG_MARKER`] hangs in it.

use std::ffi::{CStr, c_char, c_void};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard, OnceLock, PoisonError};

mod gui;

use vox_clap_abi::*;
use vox_module_api::{
    ActivateConfig, ChannelLayout, EventList, Module, OutputEvents, ParamEvent, ProcessContext,
    ProcessMode, Transport,
};
use vox_modules::Gain;

/// Mono gain.
pub const ID_GAIN: &str = "org.powervoice.test.gain";
/// Stereo-only gain.
pub const ID_GAIN_STEREO: &str = "org.powervoice.test.gain-stereo";
/// Gain with a latency parameter.
pub const ID_LATENCY: &str = "org.powervoice.test.latency";
/// Every plugin id the factory offers, in factory order.
pub const IDS: [&str; 3] = [ID_GAIN, ID_GAIN_STEREO, ID_LATENCY];
/// Fixed latency of the gain variants (and the default of `latency`).
pub const LATENCY_SAMPLES: u32 = 64;
/// Largest `latency`.
pub const MAX_LATENCY: u32 = 4096;
/// CLAP id of `gain_db`.
pub const PARAM_GAIN: u32 = 0;
/// CLAP id of `latency`.
pub const PARAM_LATENCY: u32 = 1;
/// A bundle whose file name contains this aborts in `clap_entry.init`.
pub const CRASH_ON_SCAN_MARKER: &str = "crash-on-scan";
/// A bundle whose file name contains this never returns from `clap_entry.init`.
pub const HANG_ON_SCAN_MARKER: &str = "hang-on-scan";
/// State magic.
pub const STATE_MAGIC: [u8; 4] = *b"PVTC";
/// T-901: the editor's size in pixels.
pub const GUI_WIDTH: u32 = 320;
/// T-901: the editor's size in pixels.
pub const GUI_HEIGHT: u32 = 200;
/// T-901: the gain the editor sets on its first timer tick.
pub const GUI_EDIT_GAIN_DB: f64 = -9.0;
/// T-901: the GUI-only setting the editor stores in the state (version 2) on that tick.
pub const GUI_MARKER: u32 = 0x00C0_FFEE;
/// T-901: the editor's timer period.
pub const GUI_TIMER_MS: u32 = 20;
/// T-901: a copy whose file name contains this crashes in its editor's timer.
pub const GUI_CRASH_MARKER: &str = "gui-crash";
/// T-901: a copy whose file name contains this hangs in its editor's timer.
pub const GUI_HANG_MARKER: &str = "gui-hang";

/// The file name this library was loaded as (`clap_entry.init`), for the GUI's markers.
static PLUGIN_FILE: OnceLock<String> = OnceLock::new();

#[repr(transparent)]
struct Descriptor(clap_plugin_descriptor);

// SAFETY: every pointer in these descriptors points at 'static, immutable data.
unsafe impl Sync for Descriptor {}

static FEATURES_MONO: [StaticStr; 4] = [
    StaticStr::new(c"audio-effect"),
    StaticStr::new(c"utility"),
    StaticStr::new(c"mono"),
    StaticStr::NULL,
];
static FEATURES_STEREO: [StaticStr; 4] = [
    StaticStr::new(c"audio-effect"),
    StaticStr::new(c"utility"),
    StaticStr::new(c"stereo"),
    StaticStr::NULL,
];

const fn descriptor(
    id: &'static CStr,
    name: &'static CStr,
    features: &'static [StaticStr; 4],
) -> Descriptor {
    Descriptor(clap_plugin_descriptor {
        clap_version: CLAP_VERSION,
        id: id.as_ptr(),
        name: name.as_ptr(),
        vendor: c"PowerVoice".as_ptr(),
        url: c"https://powervoice.app".as_ptr(),
        manual_url: std::ptr::null(),
        support_url: std::ptr::null(),
        version: c"1.2.3".as_ptr(),
        description: c"PowerVoice test plugin (gain + latency)".as_ptr(),
        features: features.as_ptr().cast(),
    })
}

static DESCRIPTORS: [Descriptor; 3] = [
    descriptor(c"org.powervoice.test.gain", c"Test Gain", &FEATURES_MONO),
    descriptor(
        c"org.powervoice.test.gain-stereo",
        c"Test Gain (stereo)",
        &FEATURES_STEREO,
    ),
    descriptor(
        c"org.powervoice.test.latency",
        c"Test Latency",
        &FEATURES_MONO,
    ),
];

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
}

// --- Entry and factory -----------------------------------------------------------------------

unsafe extern "C" fn entry_init(plugin_path: *const c_char) -> bool {
    if !plugin_path.is_null() {
        // SAFETY: CLAP passes a NUL-terminated path.
        let path = unsafe { CStr::from_ptr(plugin_path) }.to_string_lossy();
        let name = path.rsplit(['/', '\\']).next().unwrap_or("");
        let _ = PLUGIN_FILE.set(name.to_owned());
        if name.contains(CRASH_ON_SCAN_MARKER) {
            crash_without_core_dump();
        }
        if name.contains(HANG_ON_SCAN_MARKER) {
            loop {
                std::thread::sleep(std::time::Duration::from_secs(3600));
            }
        }
    }
    true
}

/// The deliberate "crashing bundle during scan" abort — without a core dump: an intentional
/// test crash must not reach the system's crash reporter (systemd-coredump turns every core
/// into a "Process crashed" desktop notification). The process still dies by `SIGABRT`, so the
/// scan reports a crash.
fn crash_without_core_dump() -> ! {
    #[cfg(unix)]
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

unsafe extern "C" fn entry_deinit() {}

unsafe extern "C" fn entry_get_factory(factory_id: *const c_char) -> *const c_void {
    // SAFETY: CLAP passes a NUL-terminated id.
    if !factory_id.is_null() && unsafe { CStr::from_ptr(factory_id) } == CLAP_PLUGIN_FACTORY_ID {
        (&raw const FACTORY).cast()
    } else {
        std::ptr::null()
    }
}

/// The CLAP entry point.
#[unsafe(no_mangle)]
#[allow(non_upper_case_globals)]
pub static clap_entry: clap_plugin_entry = clap_plugin_entry {
    clap_version: CLAP_VERSION,
    init: Some(entry_init),
    deinit: Some(entry_deinit),
    get_factory: Some(entry_get_factory),
};

static FACTORY: clap_plugin_factory = clap_plugin_factory {
    get_plugin_count: Some(factory_count),
    get_plugin_descriptor: Some(factory_descriptor),
    create_plugin: Some(factory_create),
};

unsafe extern "C" fn factory_count(_: *const clap_plugin_factory) -> u32 {
    DESCRIPTORS.len() as u32
}

unsafe extern "C" fn factory_descriptor(
    _: *const clap_plugin_factory,
    index: u32,
) -> *const clap_plugin_descriptor {
    DESCRIPTORS
        .get(index as usize)
        .map_or(std::ptr::null(), |d| &raw const d.0)
}

unsafe extern "C" fn factory_create(
    _: *const clap_plugin_factory,
    host: *const clap_host,
    plugin_id: *const c_char,
) -> *const clap_plugin {
    if host.is_null() || plugin_id.is_null() {
        return std::ptr::null();
    }
    // SAFETY: NUL-terminated per CLAP.
    let id = unsafe { CStr::from_ptr(plugin_id) }.to_string_lossy();
    let Some(index) = IDS.iter().position(|i| *i == id) else {
        return std::ptr::null();
    };
    let kind = [Kind::Mono, Kind::Stereo, Kind::Latency][index];
    let plugin = Box::new(Plugin {
        clap: clap_plugin {
            desc: &raw const DESCRIPTORS[index].0,
            plugin_data: std::ptr::null_mut(),
            init: Some(plugin_init),
            destroy: Some(plugin_destroy),
            activate: Some(plugin_activate),
            deactivate: Some(plugin_deactivate),
            start_processing: Some(plugin_start_processing),
            stop_processing: Some(plugin_stop_processing),
            reset: Some(plugin_reset),
            process: Some(plugin_process),
            get_extension: Some(plugin_get_extension),
            on_main_thread: Some(plugin_on_main_thread),
        },
        host,
        kind,
        gain_db: AtomicU64::new(0f64.to_bits()),
        latency_param: AtomicU32::new(LATENCY_SAMPLES),
        active_latency: AtomicU32::new(LATENCY_SAMPLES),
        audio: Mutex::new(Audio::new(kind)),
        gui: Mutex::new(None),
        gui_edit_pending: AtomicBool::new(false),
        gui_marker: AtomicU32::new(0),
        active: AtomicBool::new(false),
    });
    let raw = Box::into_raw(plugin);
    // SAFETY: `raw` is the live allocation just leaked; `clap` is its first field.
    unsafe {
        (*raw).clap.plugin_data = raw.cast();
        &raw const (*raw).clap
    }
}

// --- The plugin ------------------------------------------------------------------------------

#[repr(C)]
struct Plugin {
    clap: clap_plugin,
    host: *const clap_host,
    kind: Kind,
    /// `gain_db` as the main thread sees it (bits of an `f64`).
    gain_db: AtomicU64,
    /// `latency` as set (applies at the next activation).
    latency_param: AtomicU32,
    /// The latency of the current activation.
    active_latency: AtomicU32,
    /// Audio-side state (locked per block by the audio thread; by the main thread only while
    /// inactive or for a flush/state load).
    audio: Mutex<Audio>,
    /// T-901: the open editor (main thread).
    gui: Mutex<Option<gui::Gui>>,
    /// T-901: the editor changed `gain_db`; report it (next `process` or `flush`).
    gui_edit_pending: AtomicBool,
    /// T-901: the GUI-only setting (state version 2).
    gui_marker: AtomicU32,
    /// Between `activate` and `deactivate`.
    active: AtomicBool,
}

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

    /// Jumps every channel's gain to `db` (no ramp: inactive, flush, state).
    fn snap_gain(&mut self, db: f64) {
        for g in &mut self.gains {
            let _ = g.load_state(&Gain::state_with_gain_db(db));
        }
    }
}

/// # Safety
/// `p` is a plugin created by [`factory_create`] and not destroyed.
unsafe fn this<'a>(p: *const clap_plugin) -> &'a Plugin {
    // SAFETY: `plugin_data` points at the owning `Plugin` (caller's contract).
    unsafe { &*((*p).plugin_data as *const Plugin) }
}

impl Plugin {
    fn audio(&self) -> MutexGuard<'_, Audio> {
        self.audio.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn gui(&self) -> MutexGuard<'_, Option<gui::Gui>> {
        self.gui.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn gain_db(&self) -> f64 {
        f64::from_bits(self.gain_db.load(Ordering::Acquire))
    }

    fn set_gain_db(&self, audio: &mut Audio, db: f64, snap: bool) {
        let db = Gain::gain_param().clamp_quantize(db);
        self.gain_db.store(db.to_bits(), Ordering::Release);
        if snap {
            audio.snap_gain(db);
        }
    }

    fn set_latency(&self, value: f64) {
        let v = value.round().clamp(0.0, f64::from(MAX_LATENCY)) as u32;
        let old = self.latency_param.swap(v, Ordering::AcqRel);
        if old != v && v != self.active_latency.load(Ordering::Acquire) {
            // SAFETY: the host outlives the plugin; `request_restart` is thread-safe.
            unsafe {
                if let Some(f) = (*self.host).request_restart {
                    f(self.host);
                }
            }
        }
    }

    fn latency(&self) -> u32 {
        match self.kind {
            Kind::Latency => self.latency_param.load(Ordering::Acquire),
            _ => LATENCY_SAMPLES,
        }
    }

    fn param_count(&self) -> u32 {
        if self.kind == Kind::Latency { 2 } else { 1 }
    }

    /// Applies `in_events`' parameter values without audio (flush).
    ///
    /// # Safety
    /// `in_events` is null or a valid CLAP input event list.
    unsafe fn apply_events(&self, audio: &mut Audio, in_events: *const clap_input_events) {
        // SAFETY: forwarded contract.
        for ev in unsafe { param_values(in_events) } {
            match ev.param_id {
                PARAM_GAIN => self.set_gain_db(audio, ev.value, true),
                PARAM_LATENCY if self.kind == Kind::Latency => self.set_latency(ev.value),
                _ => {}
            }
        }
    }
}

/// The parameter-value events of a CLAP input list.
///
/// # Safety
/// `list` is null or a valid CLAP input event list for the duration of the iteration.
unsafe fn param_values<'a>(
    list: *const clap_input_events,
) -> impl Iterator<Item = &'a clap_event_param_value> {
    // SAFETY: caller's contract.
    let n = if list.is_null() {
        0
    } else {
        unsafe { (*list).size.map_or(0, |f| f(list)) }
    };
    (0..n).filter_map(move |i| {
        // SAFETY: `i < size`; the host keeps the events alive during the call.
        let h = unsafe { (*list).get.map_or(std::ptr::null(), |f| f(list, i)) };
        if h.is_null() {
            return None;
        }
        // SAFETY: non-null header of a live event.
        let header = unsafe { &*h };
        (header.space_id == CLAP_CORE_EVENT_SPACE_ID && header.type_ == CLAP_EVENT_PARAM_VALUE)
            // SAFETY: the type says it is a `clap_event_param_value`.
            .then(|| unsafe { &*(h as *const clap_event_param_value) })
    })
}

unsafe extern "C" fn plugin_init(_: *const clap_plugin) -> bool {
    true
}

unsafe extern "C" fn plugin_destroy(p: *const clap_plugin) {
    // SAFETY: a live plugin (a host that didn't destroy the GUI first).
    gui::teardown(unsafe { this(p) });
    // SAFETY: `plugin_data` is the `Box` leaked in `factory_create`; destroy is called once.
    unsafe { drop(Box::from_raw((*p).plugin_data as *mut Plugin)) };
}

/// Pushes a `gain_db` change the editor made (gesture begin, value, end) to `out`.
///
/// # Safety
/// `out` is null or a valid CLAP output event list for the call.
unsafe fn report_gui_edit(out: *const clap_output_events, value: f64) {
    if out.is_null() {
        return;
    }
    // SAFETY: caller's contract.
    let Some(push) = (unsafe { (*out).try_push }) else {
        return;
    };
    let gesture = |type_: u16| clap_event_param_gesture {
        header: clap_event_header {
            size: std::mem::size_of::<clap_event_param_gesture>() as u32,
            time: 0,
            space_id: CLAP_CORE_EVENT_SPACE_ID,
            type_,
            flags: 0,
        },
        param_id: PARAM_GAIN,
    };
    let begin = gesture(CLAP_EVENT_PARAM_GESTURE_BEGIN);
    let end = gesture(CLAP_EVENT_PARAM_GESTURE_END);
    let v = clap_event_param_value {
        header: clap_event_header {
            size: std::mem::size_of::<clap_event_param_value>() as u32,
            time: 0,
            space_id: CLAP_CORE_EVENT_SPACE_ID,
            type_: CLAP_EVENT_PARAM_VALUE,
            flags: 0,
        },
        param_id: PARAM_GAIN,
        cookie: std::ptr::null_mut(),
        note_id: -1,
        port_index: -1,
        channel: -1,
        key: -1,
        value,
    };
    // SAFETY: the host's list; the events live for the calls.
    unsafe {
        push(out, &begin.header);
        push(out, &v.header);
        push(out, &end.header);
    }
}

unsafe extern "C" fn plugin_activate(
    p: *const clap_plugin,
    sample_rate: f64,
    _min_frames: u32,
    max_frames: u32,
) -> bool {
    // SAFETY: a live plugin.
    let this = unsafe { this(p) };
    let config = ActivateConfig {
        sample_rate,
        max_block: max_frames,
        mode: ProcessMode::Realtime,
        layout: ChannelLayout::MONO,
    };
    let latency = this.latency();
    let mut a = this.audio();
    for g in &mut a.gains {
        if g.activate(&config).is_err() {
            return false;
        }
    }
    let channels = this.kind.channels();
    a.delay = vec![vec![0.0; latency as usize]; channels];
    a.delay_pos = 0;
    a.scratch = vec![0.0; max_frames as usize];
    a.steady = 0;
    this.active_latency.store(latency, Ordering::Release);
    this.active.store(true, Ordering::Release);
    true
}

unsafe extern "C" fn plugin_deactivate(p: *const clap_plugin) {
    // SAFETY: a live plugin.
    let this = unsafe { this(p) };
    this.active.store(false, Ordering::Release);
    for g in &mut this.audio().gains {
        g.deactivate();
    }
}

unsafe extern "C" fn plugin_start_processing(_: *const clap_plugin) -> bool {
    true
}

unsafe extern "C" fn plugin_stop_processing(_: *const clap_plugin) {}

unsafe extern "C" fn plugin_reset(p: *const clap_plugin) {
    // SAFETY: a live plugin.
    let this = unsafe { this(p) };
    let mut a = this.audio();
    for g in &mut a.gains {
        g.reset();
    }
    for d in &mut a.delay {
        d.fill(0.0);
    }
}

unsafe extern "C" fn plugin_process(
    p: *const clap_plugin,
    process: *const clap_process,
) -> clap_process_status {
    // SAFETY: a live plugin and a valid process block for the call.
    let (this, pr) = unsafe { (this(p), &*process) };
    let n = pr.frames_count as usize;
    let channels = this.kind.channels();
    if pr.audio_inputs_count < 1 || pr.audio_outputs_count < 1 {
        return CLAP_PROCESS_ERROR;
    }
    // SAFETY: at least one port each, as checked.
    let (inp, out) = unsafe { (&*pr.audio_inputs, &*pr.audio_outputs) };
    if (inp.channel_count as usize) < channels || (out.channel_count as usize) < channels {
        return CLAP_PROCESS_ERROR;
    }
    let mut guard = this.audio();
    let a = &mut *guard;
    if n > a.scratch.len() {
        return CLAP_PROCESS_ERROR;
    }
    a.events.clear();
    // T-901: an edit the editor made applies from this block's first sample.
    let gui_edit = this
        .gui_edit_pending
        .swap(false, Ordering::AcqRel)
        .then(|| this.gain_db());
    if let Some(v) = gui_edit {
        let _ = a.events.push(ParamEvent {
            offset: 0,
            id: Gain::GAIN_DB,
            value: v,
        });
    }
    // SAFETY: the host's list, valid during the call.
    for ev in unsafe { param_values(pr.in_events) } {
        match ev.param_id {
            PARAM_GAIN => {
                let v = Gain::gain_param().clamp_quantize(ev.value);
                this.gain_db.store(v.to_bits(), Ordering::Release);
                let _ = a.events.push(ParamEvent {
                    offset: ev.header.time.min(pr.frames_count.saturating_sub(1)),
                    id: Gain::GAIN_DB,
                    value: v,
                });
            }
            PARAM_LATENCY if this.kind == Kind::Latency => this.set_latency(ev.value),
            _ => {}
        }
    }
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
        // SAFETY: the host provides `channel_count` channels of `frames_count` samples.
        let (x, y) = unsafe {
            (
                std::slice::from_raw_parts(*inp.data32.add(ch), n),
                std::slice::from_raw_parts_mut(*out.data32.add(ch), n),
            )
        };
        gain_out.clear();
        let mut ctx = ProcessContext::new(
            n as u32,
            *steady,
            Transport::default(),
            events.as_slice(),
            gain_out,
        );
        {
            let mut outs = [&mut scratch[..n]];
            gain.process(&mut ctx, &[x], &mut outs);
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
    if let Some(v) = gui_edit {
        // SAFETY: the host's output list, valid during the call.
        unsafe { report_gui_edit(pr.out_events, v) };
    }
    CLAP_PROCESS_CONTINUE
}

unsafe extern "C" fn plugin_on_main_thread(_: *const clap_plugin) {}

unsafe extern "C" fn plugin_get_extension(
    p: *const clap_plugin,
    id: *const c_char,
) -> *const c_void {
    if id.is_null() {
        return std::ptr::null();
    }
    // SAFETY: NUL-terminated per CLAP; `p` is live.
    let (id, this) = unsafe { (CStr::from_ptr(id), this(p)) };
    let _ = this;
    if id == CLAP_EXT_GUI {
        return (&raw const gui::GUI).cast();
    }
    if id == CLAP_EXT_TIMER_SUPPORT {
        return (&raw const gui::TIMER_SUPPORT).cast();
    }
    if cfg!(unix) && id == CLAP_EXT_POSIX_FD_SUPPORT {
        return (&raw const gui::POSIX_FD_SUPPORT).cast();
    }
    if id == CLAP_EXT_PARAMS {
        (&raw const PARAMS).cast()
    } else if id == CLAP_EXT_STATE {
        (&raw const STATE).cast()
    } else if id == CLAP_EXT_LATENCY {
        (&raw const LATENCY).cast()
    } else if id == CLAP_EXT_TAIL {
        (&raw const TAIL).cast()
    } else if id == CLAP_EXT_AUDIO_PORTS {
        (&raw const AUDIO_PORTS).cast()
    } else {
        std::ptr::null()
    }
}

// --- Extensions ------------------------------------------------------------------------------

static PARAMS: clap_plugin_params = clap_plugin_params {
    count: Some(params_count),
    get_info: Some(params_get_info),
    get_value: Some(params_get_value),
    value_to_text: Some(params_value_to_text),
    text_to_value: Some(params_text_to_value),
    flush: Some(params_flush),
};

unsafe extern "C" fn params_count(p: *const clap_plugin) -> u32 {
    // SAFETY: a live plugin.
    unsafe { this(p) }.param_count()
}

unsafe extern "C" fn params_get_info(
    p: *const clap_plugin,
    index: u32,
    info: *mut clap_param_info,
) -> bool {
    // SAFETY: a live plugin; `info` is writable.
    let (this, info) = unsafe { (this(p), &mut *info) };
    if index >= this.param_count() {
        return false;
    }
    info.cookie = std::ptr::null_mut();
    write_fixed_str(&mut info.module, "");
    if index == 0 {
        info.id = PARAM_GAIN;
        info.flags = CLAP_PARAM_IS_AUTOMATABLE;
        write_fixed_str(&mut info.name, "Gain");
        (info.min_value, info.max_value, info.default_value) = (-60.0, 24.0, 0.0);
    } else {
        info.id = PARAM_LATENCY;
        info.flags = CLAP_PARAM_IS_STEPPED;
        write_fixed_str(&mut info.name, "Latency");
        write_fixed_str(&mut info.module, "Advanced");
        (info.min_value, info.max_value, info.default_value) =
            (0.0, f64::from(MAX_LATENCY), f64::from(LATENCY_SAMPLES));
    }
    true
}

unsafe extern "C" fn params_get_value(p: *const clap_plugin, id: clap_id, out: *mut f64) -> bool {
    // SAFETY: a live plugin; `out` is writable.
    let this = unsafe { this(p) };
    let v = match id {
        PARAM_GAIN => this.gain_db(),
        PARAM_LATENCY if this.kind == Kind::Latency => {
            f64::from(this.latency_param.load(Ordering::Acquire))
        }
        _ => return false,
    };
    // SAFETY: see above.
    unsafe { *out = v };
    true
}

unsafe extern "C" fn params_value_to_text(
    p: *const clap_plugin,
    id: clap_id,
    value: f64,
    buf: *mut c_char,
    capacity: u32,
) -> bool {
    // SAFETY: a live plugin.
    let this = unsafe { this(p) };
    let text = match id {
        PARAM_GAIN => format!("{value:.2} dB"),
        PARAM_LATENCY if this.kind == Kind::Latency => format!("{} smp", value.round()),
        _ => return false,
    };
    if buf.is_null() || capacity == 0 {
        return false;
    }
    // SAFETY: the host's buffer of `capacity` chars.
    let dst = unsafe { std::slice::from_raw_parts_mut(buf, capacity as usize) };
    write_fixed_str(dst, &text);
    true
}

unsafe extern "C" fn params_text_to_value(
    p: *const clap_plugin,
    id: clap_id,
    text: *const c_char,
    out: *mut f64,
) -> bool {
    // SAFETY: a live plugin; NUL-terminated text; writable `out`.
    let (this, text) = unsafe { (this(p), CStr::from_ptr(text).to_string_lossy()) };
    let t = text.trim();
    let number = match id {
        PARAM_GAIN => t.strip_suffix("dB").unwrap_or(t),
        PARAM_LATENCY if this.kind == Kind::Latency => t.strip_suffix("smp").unwrap_or(t),
        _ => return false,
    };
    match number.trim().parse::<f64>() {
        Ok(v) if v.is_finite() => {
            // SAFETY: see above.
            unsafe { *out = v };
            true
        }
        _ => false,
    }
}

unsafe extern "C" fn params_flush(
    p: *const clap_plugin,
    in_events: *const clap_input_events,
    out: *const clap_output_events,
) {
    // SAFETY: a live plugin; the host's event list.
    let this = unsafe { this(p) };
    let mut a = this.audio();
    // SAFETY: forwarded.
    unsafe { this.apply_events(&mut a, in_events) };
    // T-901: an edit the editor made while inactive.
    if this.gui_edit_pending.swap(false, Ordering::AcqRel) {
        let v = this.gain_db();
        a.snap_gain(v);
        // SAFETY: the host's output list, valid during the call.
        unsafe { report_gui_edit(out, v) };
    }
}

static STATE: clap_plugin_state = clap_plugin_state {
    save: Some(state_save),
    load: Some(state_load),
};

unsafe extern "C" fn state_save(p: *const clap_plugin, stream: *const clap_ostream) -> bool {
    // SAFETY: a live plugin.
    let this = unsafe { this(p) };
    let mut bytes = Vec::with_capacity(24);
    bytes.extend_from_slice(&STATE_MAGIC);
    bytes.extend_from_slice(&2u32.to_le_bytes());
    bytes.extend_from_slice(&this.gain_db().to_le_bytes());
    bytes.extend_from_slice(&this.latency_param.load(Ordering::Acquire).to_le_bytes());
    bytes.extend_from_slice(&this.gui_marker.load(Ordering::Acquire).to_le_bytes());
    let mut rest = bytes.as_slice();
    while !rest.is_empty() {
        // SAFETY: the host's stream, valid during the call.
        let n = unsafe {
            match (*stream).write {
                Some(w) => w(stream, rest.as_ptr().cast(), rest.len() as u64),
                None => -1,
            }
        };
        if n <= 0 {
            return false;
        }
        rest = &rest[(n as usize).min(rest.len())..];
    }
    true
}

unsafe extern "C" fn state_load(p: *const clap_plugin, stream: *const clap_istream) -> bool {
    // SAFETY: a live plugin.
    let this = unsafe { this(p) };
    let mut bytes = Vec::new();
    let mut chunk = [0u8; 7];
    loop {
        // SAFETY: the host's stream; `chunk` is writable.
        let n = unsafe {
            match (*stream).read {
                Some(r) => r(stream, chunk.as_mut_ptr().cast(), chunk.len() as u64),
                None => -1,
            }
        };
        match n {
            0 => break,
            n if n < 0 => return false,
            n => bytes.extend_from_slice(&chunk[..(n as usize).min(chunk.len())]),
        }
    }
    // Version 1 (T-803: 20 bytes) or 2 (T-901: + the GUI marker).
    let v1 = bytes.len() == 20 && bytes[4..8] == 1u32.to_le_bytes();
    let v2 = bytes.len() == 24 && bytes[4..8] == 2u32.to_le_bytes();
    if !(v1 || v2) || bytes[..4] != STATE_MAGIC {
        return false;
    }
    let gain = f64::from_le_bytes(bytes[8..16].try_into().unwrap_or_default());
    let latency = u32::from_le_bytes(bytes[16..20].try_into().unwrap_or_default());
    let marker = if v2 {
        u32::from_le_bytes(bytes[20..24].try_into().unwrap_or_default())
    } else {
        0
    };
    this.gui_marker.store(marker, Ordering::Release);
    let mut a = this.audio();
    this.set_gain_db(&mut a, gain, true);
    if this.kind == Kind::Latency {
        this.latency_param
            .store(latency.min(MAX_LATENCY), Ordering::Release);
    }
    true
}

static LATENCY: clap_plugin_latency = clap_plugin_latency {
    get: Some(latency_get),
};

unsafe extern "C" fn latency_get(p: *const clap_plugin) -> u32 {
    // SAFETY: a live plugin.
    unsafe { this(p) }.active_latency.load(Ordering::Acquire)
}

static TAIL: clap_plugin_tail = clap_plugin_tail {
    get: Some(tail_get),
};

unsafe extern "C" fn tail_get(_: *const clap_plugin) -> u32 {
    0
}

static AUDIO_PORTS: clap_plugin_audio_ports = clap_plugin_audio_ports {
    count: Some(ports_count),
    get: Some(ports_get),
};

unsafe extern "C" fn ports_count(_: *const clap_plugin, _is_input: bool) -> u32 {
    1
}

unsafe extern "C" fn ports_get(
    p: *const clap_plugin,
    index: u32,
    _is_input: bool,
    info: *mut clap_audio_port_info,
) -> bool {
    if index != 0 {
        return false;
    }
    // SAFETY: a live plugin; `info` is writable.
    let (this, info) = unsafe { (this(p), &mut *info) };
    info.id = 0;
    write_fixed_str(&mut info.name, "Main");
    info.flags = CLAP_AUDIO_PORT_IS_MAIN;
    info.channel_count = this.kind.channels() as u32;
    info.port_type = if this.kind == Kind::Stereo {
        CLAP_PORT_STEREO.as_ptr()
    } else {
        CLAP_PORT_MONO.as_ptr()
    };
    info.in_place_pair = CLAP_INVALID_ID;
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descriptor_ids_match_the_rust_constants() {
        for (d, id) in DESCRIPTORS.iter().zip(IDS) {
            // SAFETY: static descriptor strings.
            let got = unsafe { CStr::from_ptr(d.0.id) }.to_str().unwrap();
            assert_eq!(got, id);
        }
    }
}

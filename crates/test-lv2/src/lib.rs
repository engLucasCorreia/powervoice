//! The in-repo **test LV2 plugin** (T-807): a `cdylib` exporting `lv2_descriptor`, used by the
//! sandbox's LV2 backend tests instead of installed plugins, and [`write_bundle`], which writes a
//! complete `.lv2` bundle (`manifest.ttl`, `plugins.ttl`, a copy of the library) so tests go
//! through real lilv discovery. Its library offers:
//!
//! | URI | Ports | Features | Latency |
//! |---|---|---|---|
//! | [`URI_GAIN`] | mono in/out, `gain`, `mode` (enumeration), `toggle`, `freq` (log), `level` (output), atom in/out | requires `urid:map`, `options:options`, `bufsz:boundedBlockLength`; `state:interface` | [`LATENCY_SAMPLES`] |
//! | [`URI_GAIN_STEREO`] | **stereo only** in/out (the host's mono shim), `gain` | none; no state interface (the host saves the control values) | [`LATENCY_SAMPLES`] |
//! | [`URI_LATENCY`] | mono in/out, `gain`, `latency` (integer, "Advanced" group) | requires `urid:map`, `worker:schedule`; `worker:interface`, `state:interface` | the `latency` value at activation |
//!
//! Every variant runs the built-in Gain (`vox_modules::Gain`, bit-exact with the in-process
//! module) per channel, then a delay line of its latency, and reports that latency on its
//! `lv2:latency` output port. The gain port is read at the start of every `run`: a new value is a
//! Gain event at offset 0 (the host splits blocks at its own event offsets, so this is
//! sample-accurate); `activate` snaps the Gain to the port's value.
//!
//! - **Worker:** when the latency variant's `latency` port changes, `run` schedules work; `work`
//!   answers with the same value and `work_response` makes it the *reported* latency (the delay
//!   line keeps the activation's), so the host only sees the change after a real worker round
//!   trip — and asks for a restart.
//! - **Options:** the mono gain refuses to instantiate without `bufsz:maxBlockLength` and
//!   `param:sampleRate` (equal to the instantiation rate) among the options.
//! - **Atom ports:** the mono gain outputs silence unless its atom input holds a valid (empty)
//!   `atom:Sequence`; it writes an empty sequence to its atom output.
//! - **State:** [`STATE_KEY_RESTORES`] (`atom:Int`: 0 when fresh, the stored value + 1 after a
//!   restore) and [`STATE_KEY_NOTE`] (`atom:String`).
//! - A bundle whose path contains [`CRASH_ON_SCAN_MARKER`] aborts in `instantiate` (the
//!   "crashing bundle during scan" test — the scan instantiates effects); [`HANG_ON_SCAN_MARKER`]
//!   never returns from it.

use std::ffi::{CStr, CString, c_char, c_void};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI32, AtomicU32, Ordering};

use vox_lv2_abi::*;
use vox_module_api::{
    ActivateConfig, ChannelLayout, EventList, Module, OutputEvents, ParamEvent, ProcessContext,
    ProcessMode, Transport,
};
use vox_modules::Gain;

/// Mono gain.
pub const URI_GAIN: &str = "https://powervoice.app/test/lv2/gain";
/// Stereo-only gain.
pub const URI_GAIN_STEREO: &str = "https://powervoice.app/test/lv2/gain-stereo";
/// Gain with a latency port.
pub const URI_LATENCY: &str = "https://powervoice.app/test/lv2/latency";
/// Every plugin URI the library offers, in descriptor order.
pub const URIS: [&str; 3] = [URI_GAIN, URI_GAIN_STEREO, URI_LATENCY];
/// Fixed latency of the gain variants (and the default of `latency`).
pub const LATENCY_SAMPLES: u32 = 64;
/// Largest `latency`.
pub const MAX_LATENCY: u32 = 4096;
/// Port index of `gain` (dB) in every variant (the host's parameter id).
pub const PORT_GAIN: u32 = 2;
/// The mono gain's `mode` port (an enumeration: Clean, Warm, Hot; no audible effect).
pub const PORT_MODE: u32 = 6;
/// The mono gain's `toggle` port (`lv2:toggled`; no audible effect).
pub const PORT_TOGGLE: u32 = 7;
/// The mono gain's `freq` port (logarithmic, Hz; no audible effect).
pub const PORT_FREQ: u32 = 8;
/// The mono gain's `level` output port (echoes `gain`).
pub const PORT_LEVEL: u32 = 9;
/// The latency variant's `latency` input port.
pub const PORT_LATENCY: u32 = 3;
/// A bundle whose path contains this aborts in `instantiate`.
pub const CRASH_ON_SCAN_MARKER: &str = "crash-on-scan";
/// A bundle whose path contains this never returns from `instantiate`.
pub const HANG_ON_SCAN_MARKER: &str = "hang-on-scan";
/// State property: how often this lineage of states was restored (`atom:Int`).
pub const STATE_KEY_RESTORES: &str = "https://powervoice.app/test/lv2#restores";
/// State property: a fixed note (`atom:String`).
pub const STATE_KEY_NOTE: &str = "https://powervoice.app/test/lv2#note";
/// The note's text.
pub const STATE_NOTE: &str = "PowerVoice test state";

const DEFAULT_SCRATCH: usize = 8192;

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

    fn port_count(self) -> usize {
        match self {
            Kind::Mono => 10,
            Kind::Stereo => 6,
            Kind::Latency => 5,
        }
    }

    /// `(inputs, outputs)` audio port indices per channel.
    fn audio_ports(self) -> (&'static [u32], &'static [u32]) {
        match self {
            Kind::Stereo => (&[0, 3], &[1, 4]),
            _ => (&[0], &[1]),
        }
    }

    /// The `lv2:latency` output port.
    fn latency_out(self) -> u32 {
        match self {
            Kind::Mono => 3,
            Kind::Stereo => 5,
            Kind::Latency => 4,
        }
    }
}

#[repr(transparent)]
struct Descriptor(LV2_Descriptor);

// SAFETY: the descriptors point at 'static, immutable data and functions.
unsafe impl Sync for Descriptor {}

/// A variant's `extension_data` (LV2 passes no instance, so each descriptor has its own).
type ExtensionData = unsafe extern "C" fn(uri: *const c_char) -> *const c_void;

const fn descriptor(uri: &'static CStr, extension_data: ExtensionData) -> Descriptor {
    Descriptor(LV2_Descriptor {
        URI: uri.as_ptr(),
        instantiate: Some(instantiate),
        connect_port: Some(connect_port),
        activate: Some(activate),
        run: Some(run),
        deactivate: Some(deactivate),
        cleanup: Some(cleanup),
        extension_data: Some(extension_data),
    })
}

static DESCRIPTORS: [Descriptor; 3] = [
    descriptor(c"https://powervoice.app/test/lv2/gain", extension_data_mono),
    descriptor(
        c"https://powervoice.app/test/lv2/gain-stereo",
        extension_data_none,
    ),
    descriptor(
        c"https://powervoice.app/test/lv2/latency",
        extension_data_latency,
    ),
];

/// The LV2 entry point.
///
/// # Safety
/// Any index is valid (out of range → null).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lv2_descriptor(index: u32) -> *const LV2_Descriptor {
    DESCRIPTORS
        .get(index as usize)
        .map_or(std::ptr::null(), |d| &raw const d.0)
}

/// The URIDs the plugin needs.
#[derive(Clone, Copy, Default)]
struct Urids {
    sequence: u32,
    int: u32,
    string: u32,
    restores: u32,
    note: u32,
}

struct Plugin {
    kind: Kind,
    ports: [*mut f32; 10],
    atom_in: *const LV2_Atom_Sequence,
    atom_out: *mut LV2_Atom_Sequence,
    urids: Urids,
    schedule: *const LV2_Worker_Schedule,
    gains: Vec<Gain>,
    events: EventList,
    gain_out: OutputEvents,
    last_gain: f32,
    delay: Vec<Vec<f32>>,
    delay_pos: usize,
    scratch: Vec<f32>,
    sample_rate: f64,
    steady: u64,
    /// The latency of the current activation (the delay line's length).
    active_latency: u32,
    /// The latency variant's last scheduled `latency` value.
    requested_latency: u32,
    /// What `work_response` made the reported latency (the worker round trip).
    reported_latency: AtomicU32,
    restores: AtomicI32,
}

/// # Safety
/// `h` is a handle `instantiate` returned and `cleanup` hasn't freed.
unsafe fn this<'a>(h: LV2_Handle) -> &'a mut Plugin {
    // SAFETY: caller's contract.
    unsafe { &mut *(h as *mut Plugin) }
}

/// The deliberate "crashing bundle during scan" abort — without a core dump (the system's crash
/// reporter must not see intentional test crashes). The process still dies by `SIGABRT`.
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

/// The feature `uri` of a host's null-terminated feature array.
///
/// # Safety
/// `features` is null or a valid null-terminated array of valid features.
unsafe fn feature(features: *const *const LV2_Feature, uri: &CStr) -> Option<*mut c_void> {
    if features.is_null() {
        return None;
    }
    let mut k = 0;
    loop {
        // SAFETY: caller's contract; the array ends with a null pointer.
        let f = unsafe { *features.add(k) };
        if f.is_null() {
            return None;
        }
        // SAFETY: a valid feature with a NUL-terminated URI.
        if unsafe { CStr::from_ptr((*f).URI) } == uri {
            // SAFETY: as above.
            return Some(unsafe { (*f).data });
        }
        k += 1;
    }
}

/// # Safety
/// `map` is a valid `LV2_URID_Map`.
unsafe fn map(map: *const LV2_URID_Map, uri: &CStr) -> u32 {
    // SAFETY: caller's contract.
    unsafe { (*map).map.map_or(0, |f| f((*map).handle, uri.as_ptr())) }
}

unsafe extern "C" fn instantiate(
    descriptor: *const LV2_Descriptor,
    sample_rate: f64,
    bundle_path: *const c_char,
    features: *const *const LV2_Feature,
) -> LV2_Handle {
    if !bundle_path.is_null() {
        // SAFETY: LV2 passes a NUL-terminated path.
        let path = unsafe { CStr::from_ptr(bundle_path) }.to_string_lossy();
        if path.contains(CRASH_ON_SCAN_MARKER) {
            crash_without_core_dump();
        }
        if path.contains(HANG_ON_SCAN_MARKER) {
            loop {
                std::thread::sleep(std::time::Duration::from_secs(3600));
            }
        }
    }
    let Some(index) = DESCRIPTORS
        .iter()
        .position(|d| std::ptr::eq(&d.0, descriptor))
    else {
        return std::ptr::null_mut();
    };
    let kind = [Kind::Mono, Kind::Stereo, Kind::Latency][index];
    let mut urids = Urids::default();
    let mut scratch = DEFAULT_SCRATCH;
    let mut schedule = std::ptr::null();
    if kind != Kind::Stereo {
        // SAFETY: the host's feature array.
        let Some(m) = (unsafe { feature(features, uri::URID_MAP) }) else {
            return std::ptr::null_mut();
        };
        let m = m as *const LV2_URID_Map;
        // SAFETY: a valid map feature.
        unsafe {
            urids = Urids {
                sequence: map(m, uri::ATOM_SEQUENCE),
                int: map(m, uri::ATOM_INT),
                string: map(m, uri::ATOM_STRING),
                restores: map(m, &CString::new(STATE_KEY_RESTORES).unwrap_or_default()),
                note: map(m, &CString::new(STATE_KEY_NOTE).unwrap_or_default()),
            };
        }
        if kind == Kind::Mono {
            // SAFETY: the host's feature array.
            let Some(opts) = (unsafe { feature(features, uri::OPTIONS_OPTIONS) }) else {
                return std::ptr::null_mut();
            };
            // SAFETY: a valid map feature.
            let (max_key, rate_key, float) = unsafe {
                (
                    map(m, uri::BUF_SIZE_MAX_BLOCK_LENGTH),
                    map(m, uri::PARAM_SAMPLE_RATE),
                    map(m, uri::ATOM_FLOAT),
                )
            };
            let (mut max_block, mut rate) = (None, None);
            let mut o = opts as *const LV2_Options_Option;
            // SAFETY: the options array ends with an entry whose key is 0 and value null.
            unsafe {
                while (*o).key != 0 || !(*o).value.is_null() {
                    if (*o).key == max_key && (*o).type_ == urids.int && (*o).size == 4 {
                        max_block = Some(*((*o).value as *const i32));
                    } else if (*o).key == rate_key && (*o).type_ == float && (*o).size == 4 {
                        rate = Some(*((*o).value as *const f32));
                    }
                    o = o.add(1);
                }
            }
            let (Some(max_block), Some(rate)) = (max_block, rate) else {
                return std::ptr::null_mut();
            };
            if max_block < 1 || rate.to_bits() != (sample_rate as f32).to_bits() {
                return std::ptr::null_mut();
            }
            scratch = max_block as usize;
        }
        if kind == Kind::Latency {
            // SAFETY: the host's feature array.
            let Some(s) = (unsafe { feature(features, uri::WORKER_SCHEDULE) }) else {
                return std::ptr::null_mut();
            };
            schedule = s as *const LV2_Worker_Schedule;
        }
    }
    let plugin = Box::new(Plugin {
        kind,
        ports: [std::ptr::null_mut(); 10],
        atom_in: std::ptr::null(),
        atom_out: std::ptr::null_mut(),
        urids,
        schedule,
        gains: (0..kind.channels()).map(|_| Gain::new()).collect(),
        events: EventList::with_capacity(vox_module_api::DEFAULT_EVENT_CAPACITY),
        gain_out: OutputEvents::with_capacity(vox_module_api::DEFAULT_EVENT_CAPACITY),
        last_gain: 0.0,
        delay: Vec::new(),
        delay_pos: 0,
        scratch: vec![0.0; scratch],
        sample_rate,
        steady: 0,
        active_latency: LATENCY_SAMPLES,
        requested_latency: LATENCY_SAMPLES,
        reported_latency: AtomicU32::new(LATENCY_SAMPLES),
        restores: AtomicI32::new(0),
    });
    Box::into_raw(plugin).cast()
}

unsafe extern "C" fn connect_port(h: LV2_Handle, port: u32, data: *mut c_void) {
    // SAFETY: a live instance.
    let p = unsafe { this(h) };
    match (p.kind, port) {
        (Kind::Mono, 4) => p.atom_in = data as *const LV2_Atom_Sequence,
        (Kind::Mono, 5) => p.atom_out = data as *mut LV2_Atom_Sequence,
        (k, i) if (i as usize) < k.port_count() => p.ports[i as usize] = data as *mut f32,
        _ => {}
    }
}

impl Plugin {
    /// A control port's value (0 when unconnected).
    fn control(&self, port: u32) -> f32 {
        let ptr = self.ports[port as usize];
        if ptr.is_null() {
            0.0
        } else {
            // SAFETY: a connected control port holds one float.
            unsafe { *ptr }
        }
    }

    fn set_output(&self, port: u32, value: f32) {
        let ptr = self.ports[port as usize];
        if !ptr.is_null() {
            // SAFETY: a connected control port holds one float.
            unsafe { *ptr = value };
        }
    }

    fn latency_port(&self) -> u32 {
        (self.control(PORT_LATENCY).round().max(0.0) as u32).min(MAX_LATENCY)
    }
}

unsafe extern "C" fn activate(h: LV2_Handle) {
    // SAFETY: a live instance.
    let p = unsafe { this(h) };
    let config = ActivateConfig {
        sample_rate: p.sample_rate,
        max_block: p.scratch.len() as u32,
        mode: ProcessMode::Realtime,
        layout: ChannelLayout::MONO,
    };
    let gain = Gain::gain_param().clamp_quantize(f64::from(p.control(PORT_GAIN)));
    for g in &mut p.gains {
        let _ = g.load_state(&Gain::state_with_gain_db(gain));
        let _ = g.activate(&config);
    }
    p.last_gain = p.control(PORT_GAIN);
    p.active_latency = if p.kind == Kind::Latency {
        p.latency_port()
    } else {
        LATENCY_SAMPLES
    };
    p.requested_latency = p.active_latency;
    p.reported_latency
        .store(p.active_latency, Ordering::Release);
    p.delay = vec![vec![0.0; p.active_latency as usize]; p.kind.channels()];
    p.delay_pos = 0;
    p.steady = 0;
}

unsafe extern "C" fn run(h: LV2_Handle, sample_count: u32) {
    // SAFETY: a live, activated instance.
    let p = unsafe { this(h) };
    let n = sample_count as usize;
    let (ins, outs) = p.kind.audio_ports();
    let mut valid = n <= p.scratch.len()
        && ins
            .iter()
            .chain(outs)
            .all(|&i| !p.ports[i as usize].is_null());
    if p.kind == Kind::Mono {
        // The host must give an (empty) atom:Sequence input and room for an output sequence.
        // SAFETY: connected atom buffers (checked for null).
        unsafe {
            valid &= !p.atom_in.is_null()
                && (*p.atom_in).atom.type_ == p.urids.sequence
                && (*p.atom_in).atom.size as usize >= std::mem::size_of::<LV2_Atom_Sequence_Body>();
            if !p.atom_out.is_null()
                && (*p.atom_out).atom.size as usize >= std::mem::size_of::<LV2_Atom_Sequence_Body>()
            {
                (*p.atom_out).atom = LV2_Atom {
                    size: std::mem::size_of::<LV2_Atom_Sequence_Body>() as u32,
                    type_: p.urids.sequence,
                };
                (*p.atom_out).body = LV2_Atom_Sequence_Body::default();
            }
        }
    }
    if !valid {
        for &o in outs {
            let ptr = p.ports[o as usize];
            if !ptr.is_null() {
                // SAFETY: the host's output buffer of `n` frames.
                unsafe { std::slice::from_raw_parts_mut(ptr, n) }.fill(0.0);
            }
        }
        return;
    }
    p.events.clear();
    let gain_port = p.control(PORT_GAIN);
    if gain_port.to_bits() != p.last_gain.to_bits() {
        p.last_gain = gain_port;
        let _ = p.events.push(ParamEvent {
            offset: 0,
            id: Gain::GAIN_DB,
            value: Gain::gain_param().clamp_quantize(f64::from(gain_port)),
        });
    }
    if p.kind == Kind::Latency && !p.schedule.is_null() {
        let v = p.latency_port();
        if v != p.requested_latency {
            let bytes = v.to_le_bytes();
            // SAFETY: the host's schedule feature (valid for the instance's lifetime).
            let status = unsafe {
                (*p.schedule)
                    .schedule_work
                    .map_or(LV2_WORKER_ERR_UNKNOWN, |f| {
                        f((*p.schedule).handle, 4, bytes.as_ptr().cast())
                    })
            };
            if status == LV2_WORKER_SUCCESS {
                p.requested_latency = v;
            }
        }
    }
    let Plugin {
        ports,
        gains,
        events,
        gain_out,
        delay,
        delay_pos,
        scratch,
        steady,
        ..
    } = p;
    let start_pos = *delay_pos;
    for (ch, gain) in gains.iter_mut().enumerate() {
        // SAFETY: connected audio ports of at least `n` frames (the host's contract); input and
        // output buffers are distinct.
        let (x, y) = unsafe {
            (
                std::slice::from_raw_parts(ports[ins[ch] as usize], n),
                std::slice::from_raw_parts_mut(ports[outs[ch] as usize], n),
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
    let reported = p.reported_latency.load(Ordering::Acquire);
    p.set_output(p.kind.latency_out(), reported as f32);
    if p.kind == Kind::Mono {
        p.set_output(PORT_LEVEL, gain_port);
    }
}

unsafe extern "C" fn deactivate(h: LV2_Handle) {
    // SAFETY: a live instance.
    let p = unsafe { this(h) };
    for g in &mut p.gains {
        g.deactivate();
    }
}

unsafe extern "C" fn cleanup(h: LV2_Handle) {
    // SAFETY: the handle `instantiate` leaked; `cleanup` is called once.
    unsafe { drop(Box::from_raw(h as *mut Plugin)) };
}

/// The extension `uri` among `state` / `worker` (null when absent or not offered).
///
/// # Safety
/// `uri` is null or NUL-terminated.
unsafe fn extension(uri: *const c_char, state: bool, worker: bool) -> *const c_void {
    if uri.is_null() {
        return std::ptr::null();
    }
    // SAFETY: NUL-terminated per LV2.
    let uri = unsafe { CStr::from_ptr(uri) };
    if state && uri == uri::STATE_INTERFACE {
        (&raw const STATE).cast()
    } else if worker && uri == uri::WORKER_INTERFACE {
        (&raw const WORKER).cast()
    } else {
        std::ptr::null()
    }
}

unsafe extern "C" fn extension_data_mono(uri: *const c_char) -> *const c_void {
    // SAFETY: LV2's contract.
    unsafe { extension(uri, true, false) }
}

unsafe extern "C" fn extension_data_none(uri: *const c_char) -> *const c_void {
    // SAFETY: LV2's contract.
    unsafe { extension(uri, false, false) }
}

unsafe extern "C" fn extension_data_latency(uri: *const c_char) -> *const c_void {
    // SAFETY: LV2's contract.
    unsafe { extension(uri, true, true) }
}

// --- Worker ------------------------------------------------------------------------------------

static WORKER: LV2_Worker_Interface = LV2_Worker_Interface {
    work: Some(work),
    work_response: Some(work_response),
    end_run: Some(end_run),
};

unsafe extern "C" fn work(
    _h: LV2_Handle,
    respond: LV2_Worker_Respond_Function,
    handle: *mut c_void,
    size: u32,
    data: *const c_void,
) -> u32 {
    if size != 4 || data.is_null() {
        return LV2_WORKER_ERR_UNKNOWN;
    }
    // SAFETY: the host's respond function with its handle; `data` holds `size` bytes.
    unsafe { respond(handle, size, data) }
}

unsafe extern "C" fn work_response(h: LV2_Handle, size: u32, body: *const c_void) -> u32 {
    if size != 4 || body.is_null() {
        return LV2_WORKER_ERR_UNKNOWN;
    }
    // SAFETY: a live instance; `body` holds 4 bytes (possibly unaligned).
    let (p, v) = unsafe { (this(h), std::ptr::read_unaligned(body as *const u32)) };
    p.reported_latency.store(v, Ordering::Release);
    LV2_WORKER_SUCCESS
}

unsafe extern "C" fn end_run(_h: LV2_Handle) -> u32 {
    LV2_WORKER_SUCCESS
}

// --- State -------------------------------------------------------------------------------------

static STATE: LV2_State_Interface = LV2_State_Interface {
    save: Some(state_save),
    restore: Some(state_restore),
};

unsafe extern "C" fn state_save(
    h: LV2_Handle,
    store: LV2_State_Store_Function,
    handle: *mut c_void,
    _flags: u32,
    _features: *const *const LV2_Feature,
) -> u32 {
    // SAFETY: a live instance.
    let p = unsafe { this(h) };
    let restores = p.restores.load(Ordering::Acquire);
    let note = CString::new(STATE_NOTE).unwrap_or_default();
    let flags = LV2_STATE_IS_POD | LV2_STATE_IS_PORTABLE;
    // SAFETY: the host's store function with its handle; the values live for the calls.
    unsafe {
        let a = store(
            handle,
            p.urids.restores,
            (&raw const restores).cast(),
            4,
            p.urids.int,
            flags,
        );
        let b = store(
            handle,
            p.urids.note,
            note.as_ptr().cast(),
            note.as_bytes_with_nul().len(),
            p.urids.string,
            flags,
        );
        if a != LV2_STATE_SUCCESS { a } else { b }
    }
}

unsafe extern "C" fn state_restore(
    h: LV2_Handle,
    retrieve: LV2_State_Retrieve_Function,
    handle: *mut c_void,
    _flags: u32,
    _features: *const *const LV2_Feature,
) -> u32 {
    // SAFETY: a live instance.
    let p = unsafe { this(h) };
    let (mut size, mut type_, mut flags) = (0usize, 0u32, 0u32);
    // SAFETY: the host's retrieve function with its handle; out-params are writable.
    let value = unsafe { retrieve(handle, p.urids.restores, &mut size, &mut type_, &mut flags) };
    if value.is_null() || size != 4 || type_ != p.urids.int {
        return LV2_STATE_ERR_NO_PROPERTY;
    }
    // SAFETY: 4 bytes the host holds for the call (possibly unaligned).
    let stored = unsafe { std::ptr::read_unaligned(value as *const i32) };
    p.restores
        .store(stored.saturating_add(1), Ordering::Release);
    LV2_STATE_SUCCESS
}

// --- The bundle --------------------------------------------------------------------------------

/// The library's file name inside a bundle called `name`.
pub fn binary_name(name: &str) -> String {
    format!("{name}{}", std::env::consts::DLL_SUFFIX)
}

fn num(v: f64) -> String {
    let s = format!("{v:?}");
    if s.contains('.') || s.contains('e') {
        s
    } else {
        format!("{s}.0")
    }
}

/// `plugins.ttl`: the three plugins' descriptions.
pub fn plugins_ttl() -> String {
    let g = Gain::gain_param();
    let gain_port = |index: u32| {
        format!(
            r#"[
        a lv2:InputPort, lv2:ControlPort ;
        lv2:index {index} ; lv2:symbol "gain" ; lv2:name "Gain" ;
        lv2:default {} ; lv2:minimum {} ; lv2:maximum {} ;
        units:unit units:db
    ]"#,
            num(g.default),
            num(g.min),
            num(g.max)
        )
    };
    let audio = |index: u32, dir: &str, symbol: &str, name: &str| {
        format!(
            r#"[ a lv2:AudioPort, lv2:{dir}Port ; lv2:index {index} ; lv2:symbol "{symbol}" ; lv2:name "{name}" ]"#
        )
    };
    let latency_out = |index: u32| {
        format!(
            r#"[
        a lv2:OutputPort, lv2:ControlPort ;
        lv2:index {index} ; lv2:symbol "latency_out" ; lv2:name "Latency" ;
        lv2:designation lv2:latency ; lv2:portProperty lv2:reportsLatency, lv2:integer ;
        lv2:minimum 0 ; lv2:maximum {MAX_LATENCY} ; units:unit units:frame
    ]"#
        )
    };
    let common = |name: &str| {
        format!(
            r#"doap:name "{name}" ;
    doap:maintainer <https://powervoice.app/test/lv2#author> ;
    rdfs:comment "PowerVoice test plugin (gain + latency)" ;
    lv2:minorVersion 2 ; lv2:microVersion 3 ;
    lv2:optionalFeature lv2:hardRTCapable"#
        )
    };
    format!(
        r#"@prefix atom: <http://lv2plug.in/ns/ext/atom#> .
@prefix bufsz: <http://lv2plug.in/ns/ext/buf-size#> .
@prefix doap: <http://usefulinc.com/ns/doap#> .
@prefix foaf: <http://xmlns.com/foaf/0.1/> .
@prefix lv2: <http://lv2plug.in/ns/lv2core#> .
@prefix opts: <http://lv2plug.in/ns/ext/options#> .
@prefix param: <http://lv2plug.in/ns/ext/parameters#> .
@prefix pg: <http://lv2plug.in/ns/ext/port-groups#> .
@prefix pprops: <http://lv2plug.in/ns/ext/port-props#> .
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
@prefix state: <http://lv2plug.in/ns/ext/state#> .
@prefix units: <http://lv2plug.in/ns/extensions/units#> .
@prefix urid: <http://lv2plug.in/ns/ext/urid#> .
@prefix work: <http://lv2plug.in/ns/ext/worker#> .

<https://powervoice.app/test/lv2#author>
    a foaf:Person ;
    foaf:name "PowerVoice" ;
    foaf:homepage <https://powervoice.app> .

<https://powervoice.app/test/lv2#advanced>
    a pg:Group ;
    lv2:symbol "advanced" ;
    lv2:name "Advanced" .

<{URI_GAIN}>
    a lv2:Plugin, lv2:UtilityPlugin ;
    {common_mono} ;
    lv2:requiredFeature urid:map, opts:options, bufsz:boundedBlockLength ;
    opts:supportedOption bufsz:maxBlockLength, param:sampleRate ;
    lv2:extensionData state:interface ;
    lv2:port {in0} , {out1} , {gain2} , {lat3} , [
        a lv2:InputPort, atom:AtomPort ;
        atom:bufferType atom:Sequence ;
        lv2:designation lv2:control ;
        lv2:index 4 ; lv2:symbol "control" ; lv2:name "Control"
    ] , [
        a lv2:OutputPort, atom:AtomPort ;
        atom:bufferType atom:Sequence ;
        lv2:index 5 ; lv2:symbol "notify" ; lv2:name "Notify"
    ] , [
        a lv2:InputPort, lv2:ControlPort ;
        lv2:index 6 ; lv2:symbol "mode" ; lv2:name "Mode" ;
        lv2:default 0 ; lv2:minimum 0 ; lv2:maximum 2 ;
        lv2:portProperty lv2:integer, lv2:enumeration ;
        lv2:scalePoint [ rdfs:label "Clean" ; rdf:value 0 ] ,
            [ rdfs:label "Warm" ; rdf:value 1 ] ,
            [ rdfs:label "Hot" ; rdf:value 2 ]
    ] , [
        a lv2:InputPort, lv2:ControlPort ;
        lv2:index 7 ; lv2:symbol "Toggle" ; lv2:name "Toggle" ;
        lv2:default 1 ; lv2:minimum 0 ; lv2:maximum 1 ;
        lv2:portProperty lv2:toggled
    ] , [
        a lv2:InputPort, lv2:ControlPort ;
        lv2:index 8 ; lv2:symbol "freq" ; lv2:name "Frequency" ;
        lv2:default 1000.0 ; lv2:minimum 20.0 ; lv2:maximum 20000.0 ;
        lv2:portProperty pprops:logarithmic ;
        units:unit units:hz
    ] , [
        a lv2:OutputPort, lv2:ControlPort ;
        lv2:index 9 ; lv2:symbol "level" ; lv2:name "Level" ;
        lv2:default 0.0 ; lv2:minimum {gmin} ; lv2:maximum {gmax}
    ] .

<{URI_GAIN_STEREO}>
    a lv2:Plugin, lv2:UtilityPlugin ;
    {common_stereo} ;
    lv2:port {in_l} , {out_l} , {gain_s} , {in_r} , {out_r} , {lat5} .

<{URI_LATENCY}>
    a lv2:Plugin, lv2:UtilityPlugin ;
    {common_latency} ;
    lv2:requiredFeature urid:map, work:schedule ;
    lv2:extensionData state:interface, work:interface ;
    lv2:port {in0} , {out1} , {gain2} , [
        a lv2:InputPort, lv2:ControlPort ;
        lv2:index 3 ; lv2:symbol "latency" ; lv2:name "Latency" ;
        lv2:default {LATENCY_SAMPLES} ; lv2:minimum 0 ; lv2:maximum {MAX_LATENCY} ;
        lv2:portProperty lv2:integer ;
        units:unit units:frame ;
        pg:group <https://powervoice.app/test/lv2#advanced>
    ] , {lat4} .
"#,
        common_mono = common("Test Gain"),
        common_stereo = common("Test Gain (stereo)"),
        common_latency = common("Test Latency"),
        in0 = audio(0, "Input", "in", "In"),
        out1 = audio(1, "Output", "out", "Out"),
        gain2 = gain_port(PORT_GAIN),
        lat3 = latency_out(3),
        lat4 = latency_out(4),
        lat5 = latency_out(5),
        in_l = audio(0, "Input", "in_l", "In L"),
        out_l = audio(1, "Output", "out_l", "Out L"),
        gain_s = gain_port(PORT_GAIN),
        in_r = audio(3, "Input", "in_r", "In R"),
        out_r = audio(4, "Output", "out_r", "Out R"),
        gmin = num(g.min),
        gmax = num(g.max),
    )
    .replace(
        "rdf:value",
        "<http://www.w3.org/1999/02/22-rdf-syntax-ns#value>",
    )
}

/// `manifest.ttl` for a bundle whose library is `binary`.
pub fn manifest_ttl(binary: &str) -> String {
    let mut s = String::from(
        "@prefix lv2: <http://lv2plug.in/ns/lv2core#> .\n\
         @prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .\n\n",
    );
    for uri in URIS {
        s.push_str(&format!(
            "<{uri}>\n    a lv2:Plugin ;\n    lv2:binary <{binary}> ;\n    rdfs:seeAlso <plugins.ttl> .\n\n"
        ));
    }
    s
}

/// Writes a complete bundle `<dir>/<name>.lv2` — `manifest.ttl`, `plugins.ttl` and a copy of
/// `library` (the built test plugin) — and returns its path.
pub fn write_bundle(dir: &Path, name: &str, library: &Path) -> std::io::Result<PathBuf> {
    let bundle = dir.join(format!("{name}.lv2"));
    std::fs::create_dir_all(&bundle)?;
    let binary = binary_name(name);
    std::fs::copy(library, bundle.join(&binary))?;
    std::fs::write(bundle.join("manifest.ttl"), manifest_ttl(&binary))?;
    std::fs::write(bundle.join("plugins.ttl"), plugins_ttl())?;
    Ok(bundle)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descriptor_uris_match_the_rust_constants() {
        for (d, uri) in DESCRIPTORS.iter().zip(URIS) {
            // SAFETY: static descriptor strings.
            let got = unsafe { CStr::from_ptr(d.0.URI) }.to_str().unwrap();
            assert_eq!(got, uri);
        }
    }

    #[test]
    fn the_ttl_names_every_plugin_and_the_binary() {
        let m = manifest_ttl("x.so");
        let p = plugins_ttl();
        for uri in URIS {
            assert!(m.contains(uri) && p.contains(uri), "{uri}");
        }
        assert!(m.contains("<x.so>"));
        assert!(p.contains("lv2:minimum -60.0"), "{p}");
    }
}

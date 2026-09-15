//! Hand-written Rust bindings for the subset of the **CLAP 1.2** C ABI PowerVoice uses (T-803,
//! ADR-008 Amendment 3): the entry point and plugin factory, the plugin and host vtables, the
//! process call (audio buffers, parameter events) and the `params`, `state`, `latency`, `tail`,
//! `audio-ports`, `log`, `thread-check` extensions. Both sides use them: the sandbox's CLAP
//! host (`powervoice-sandbox`) and the in-repo test plugin (`vox-test-clap`).
//!
//! The definitions mirror the CLAP headers (https://github.com/free-audio/clap, MIT — see
//! `LICENSE-CLAP` in this crate) field by field; names keep the C spelling so they can be
//! checked against the headers side by side. Function pointers are `Option<…>` (same ABI as a
//! nullable C function pointer), so a host can tolerate a plugin that leaves one null.
//! Layout tests pin the sizes of the structs on 64-bit targets.

#![allow(non_camel_case_types)]

use std::ffi::{CStr, c_char, c_int, c_void};

/// `clap_version_t`.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct clap_version {
    /// Major.
    pub major: u32,
    /// Minor.
    pub minor: u32,
    /// Revision.
    pub revision: u32,
}

/// The CLAP version these bindings follow.
pub const CLAP_VERSION: clap_version = clap_version {
    major: 1,
    minor: 2,
    revision: 0,
};

/// `clap_version_is_compatible`: every 1.x is compatible.
pub const fn clap_version_is_compatible(v: clap_version) -> bool {
    v.major >= 1
}

/// `clap_id`.
pub type clap_id = u32;
/// `CLAP_INVALID_ID`.
pub const CLAP_INVALID_ID: clap_id = u32::MAX;
/// `CLAP_NAME_SIZE`.
pub const CLAP_NAME_SIZE: usize = 256;
/// `CLAP_PATH_SIZE`.
pub const CLAP_PATH_SIZE: usize = 1024;

// --- Entry and factory ---------------------------------------------------------------------

/// `clap_plugin_entry_t`: the `clap_entry` symbol every `.clap` exports.
#[repr(C)]
pub struct clap_plugin_entry {
    /// CLAP version the plugin was built with.
    pub clap_version: clap_version,
    /// Called once after loading, with the path of the `.clap` file (bundle on macOS).
    pub init: Option<unsafe extern "C" fn(plugin_path: *const c_char) -> bool>,
    /// Called once before unloading.
    pub deinit: Option<unsafe extern "C" fn()>,
    /// A factory by id (`CLAP_PLUGIN_FACTORY_ID`), or null.
    pub get_factory: Option<unsafe extern "C" fn(factory_id: *const c_char) -> *const c_void>,
}

/// The exported symbol's name.
pub const CLAP_ENTRY_SYMBOL: &[u8] = b"clap_entry\0";

/// `CLAP_PLUGIN_FACTORY_ID`.
pub const CLAP_PLUGIN_FACTORY_ID: &CStr = c"clap.plugin-factory";

/// `clap_plugin_factory_t`.
#[repr(C)]
pub struct clap_plugin_factory {
    /// Number of plugins.
    pub get_plugin_count: Option<unsafe extern "C" fn(factory: *const clap_plugin_factory) -> u32>,
    /// Descriptor `index`.
    pub get_plugin_descriptor: Option<
        unsafe extern "C" fn(
            factory: *const clap_plugin_factory,
            index: u32,
        ) -> *const clap_plugin_descriptor,
    >,
    /// Creates plugin `plugin_id` (not yet initialized), or null.
    pub create_plugin: Option<
        unsafe extern "C" fn(
            factory: *const clap_plugin_factory,
            host: *const clap_host,
            plugin_id: *const c_char,
        ) -> *const clap_plugin,
    >,
}

/// `clap_plugin_descriptor_t`.
#[repr(C)]
pub struct clap_plugin_descriptor {
    /// CLAP version.
    pub clap_version: clap_version,
    /// Unique id (reverse DNS), mandatory.
    pub id: *const c_char,
    /// Display name, mandatory.
    pub name: *const c_char,
    /// Vendor (may be null).
    pub vendor: *const c_char,
    /// URL (may be null).
    pub url: *const c_char,
    /// Manual URL (may be null).
    pub manual_url: *const c_char,
    /// Support URL (may be null).
    pub support_url: *const c_char,
    /// Version string (may be null).
    pub version: *const c_char,
    /// Description (may be null).
    pub description: *const c_char,
    /// Null-terminated array of feature strings (may be null).
    pub features: *const *const c_char,
}

// --- Plugin and host -------------------------------------------------------------------------

/// `clap_plugin_t`.
#[repr(C)]
pub struct clap_plugin {
    /// Descriptor.
    pub desc: *const clap_plugin_descriptor,
    /// Reserved for the plugin.
    pub plugin_data: *mut c_void,
    /// \[main\] Must be called after creation.
    pub init: Option<unsafe extern "C" fn(plugin: *const clap_plugin) -> bool>,
    /// \[main\] Frees the plugin.
    pub destroy: Option<unsafe extern "C" fn(plugin: *const clap_plugin)>,
    /// \[main\] Activates for a sample rate and a block size range.
    pub activate: Option<
        unsafe extern "C" fn(
            plugin: *const clap_plugin,
            sample_rate: f64,
            min_frames_count: u32,
            max_frames_count: u32,
        ) -> bool,
    >,
    /// \[main\] Deactivates.
    pub deactivate: Option<unsafe extern "C" fn(plugin: *const clap_plugin)>,
    /// \[audio\] Before the first `process` after activation.
    pub start_processing: Option<unsafe extern "C" fn(plugin: *const clap_plugin) -> bool>,
    /// \[audio\] After the last `process`.
    pub stop_processing: Option<unsafe extern "C" fn(plugin: *const clap_plugin)>,
    /// \[audio\] Clears buffers, envelopes.
    pub reset: Option<unsafe extern "C" fn(plugin: *const clap_plugin)>,
    /// \[audio\] Processes one block.
    pub process: Option<
        unsafe extern "C" fn(
            plugin: *const clap_plugin,
            process: *const clap_process,
        ) -> clap_process_status,
    >,
    /// \[thread-safe\] An extension by id, or null.
    pub get_extension: Option<
        unsafe extern "C" fn(plugin: *const clap_plugin, id: *const c_char) -> *const c_void,
    >,
    /// \[main\] Answer to `clap_host::request_callback`.
    pub on_main_thread: Option<unsafe extern "C" fn(plugin: *const clap_plugin)>,
}

/// `clap_host_t`.
#[repr(C)]
pub struct clap_host {
    /// CLAP version.
    pub clap_version: clap_version,
    /// Reserved for the host.
    pub host_data: *mut c_void,
    /// Host name, mandatory.
    pub name: *const c_char,
    /// Vendor (may be null).
    pub vendor: *const c_char,
    /// URL (may be null).
    pub url: *const c_char,
    /// Version, mandatory.
    pub version: *const c_char,
    /// \[thread-safe\] A host extension by id, or null.
    pub get_extension: Option<
        unsafe extern "C" fn(host: *const clap_host, extension_id: *const c_char) -> *const c_void,
    >,
    /// \[thread-safe\] Deactivate and reactivate the plugin (latency or ports changed).
    pub request_restart: Option<unsafe extern "C" fn(host: *const clap_host)>,
    /// \[thread-safe\] Activate and start processing.
    pub request_process: Option<unsafe extern "C" fn(host: *const clap_host)>,
    /// \[thread-safe\] Call `on_main_thread` soon.
    pub request_callback: Option<unsafe extern "C" fn(host: *const clap_host)>,
}

// --- Process ---------------------------------------------------------------------------------

/// `clap_process_status`.
pub type clap_process_status = i32;
/// Processing failed: the output is invalid.
pub const CLAP_PROCESS_ERROR: clap_process_status = 0;
/// Keep processing.
pub const CLAP_PROCESS_CONTINUE: clap_process_status = 1;
/// Keep processing while the input isn't quiet.
pub const CLAP_PROCESS_CONTINUE_IF_NOT_QUIET: clap_process_status = 2;
/// Tail.
pub const CLAP_PROCESS_TAIL: clap_process_status = 3;
/// Sleep until the next event or non-silent input.
pub const CLAP_PROCESS_SLEEP: clap_process_status = 4;

/// `clap_audio_buffer_t`.
#[repr(C)]
pub struct clap_audio_buffer {
    /// `channel_count` pointers to 32-bit channels (or null).
    pub data32: *mut *mut f32,
    /// 64-bit channels (unused here: null).
    pub data64: *mut *mut f64,
    /// Channels.
    pub channel_count: u32,
    /// Latency from/to the hardware (0).
    pub latency: u32,
    /// Bit `k` set = channel `k` is constant (0: not used).
    pub constant_mask: u64,
}

/// `clap_process_t`.
#[repr(C)]
pub struct clap_process {
    /// Sample counter since activation (−1 if unknown).
    pub steady_time: i64,
    /// Frames in this block.
    pub frames_count: u32,
    /// Transport at sample 0 (null: free running).
    pub transport: *const c_void,
    /// Input ports.
    pub audio_inputs: *const clap_audio_buffer,
    /// Output ports.
    pub audio_outputs: *mut clap_audio_buffer,
    /// Number of input ports.
    pub audio_inputs_count: u32,
    /// Number of output ports.
    pub audio_outputs_count: u32,
    /// Events for this block, sorted by time.
    pub in_events: *const clap_input_events,
    /// Events the plugin reports.
    pub out_events: *const clap_output_events,
}

// --- Events ----------------------------------------------------------------------------------

/// `clap_event_header_t`.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct clap_event_header {
    /// Size of the whole event.
    pub size: u32,
    /// Sample offset within the block.
    pub time: u32,
    /// Event space (`CLAP_CORE_EVENT_SPACE_ID`).
    pub space_id: u16,
    /// Event type.
    pub type_: u16,
    /// `CLAP_EVENT_IS_LIVE`, `CLAP_EVENT_DONT_RECORD`.
    pub flags: u32,
}

/// `CLAP_CORE_EVENT_SPACE_ID`.
pub const CLAP_CORE_EVENT_SPACE_ID: u16 = 0;
/// `CLAP_EVENT_IS_LIVE`.
pub const CLAP_EVENT_IS_LIVE: u32 = 1 << 0;
/// `CLAP_EVENT_DONT_RECORD`.
pub const CLAP_EVENT_DONT_RECORD: u32 = 1 << 1;
/// `CLAP_EVENT_PARAM_VALUE`.
pub const CLAP_EVENT_PARAM_VALUE: u16 = 5;
/// `CLAP_EVENT_PARAM_MOD`.
pub const CLAP_EVENT_PARAM_MOD: u16 = 6;
/// `CLAP_EVENT_PARAM_GESTURE_BEGIN`.
pub const CLAP_EVENT_PARAM_GESTURE_BEGIN: u16 = 7;
/// `CLAP_EVENT_PARAM_GESTURE_END`.
pub const CLAP_EVENT_PARAM_GESTURE_END: u16 = 8;

/// `clap_event_param_value_t`.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct clap_event_param_value {
    /// Header (`type_` = `CLAP_EVENT_PARAM_VALUE`).
    pub header: clap_event_header,
    /// Parameter.
    pub param_id: clap_id,
    /// The parameter's `cookie` from `clap_param_info`, or null.
    pub cookie: *mut c_void,
    /// −1 = wildcard.
    pub note_id: i32,
    /// −1 = wildcard.
    pub port_index: i16,
    /// −1 = wildcard.
    pub channel: i16,
    /// −1 = wildcard.
    pub key: i16,
    /// Plain value.
    pub value: f64,
}

/// `clap_event_param_gesture_t`.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct clap_event_param_gesture {
    /// Header (`CLAP_EVENT_PARAM_GESTURE_BEGIN`/`_END`).
    pub header: clap_event_header,
    /// Parameter.
    pub param_id: clap_id,
}

/// `clap_input_events_t`.
#[repr(C)]
pub struct clap_input_events {
    /// Reserved for the host.
    pub ctx: *mut c_void,
    /// Number of events.
    pub size: Option<unsafe extern "C" fn(list: *const clap_input_events) -> u32>,
    /// Event `index` (never null for `index < size`).
    pub get: Option<
        unsafe extern "C" fn(
            list: *const clap_input_events,
            index: u32,
        ) -> *const clap_event_header,
    >,
}

/// `clap_output_events_t`.
#[repr(C)]
pub struct clap_output_events {
    /// Reserved for the host.
    pub ctx: *mut c_void,
    /// Pushes a copy of `event`; `false` if the host couldn't take it.
    pub try_push: Option<
        unsafe extern "C" fn(
            list: *const clap_output_events,
            event: *const clap_event_header,
        ) -> bool,
    >,
}

// --- Streams ---------------------------------------------------------------------------------

/// `clap_istream_t`.
#[repr(C)]
pub struct clap_istream {
    /// Reserved for the host.
    pub ctx: *mut c_void,
    /// Reads up to `size` bytes; returns the count, 0 at the end, −1 on error.
    pub read: Option<
        unsafe extern "C" fn(stream: *const clap_istream, buffer: *mut c_void, size: u64) -> i64,
    >,
}

/// `clap_ostream_t`.
#[repr(C)]
pub struct clap_ostream {
    /// Reserved for the host.
    pub ctx: *mut c_void,
    /// Writes up to `size` bytes; returns the count, −1 on error.
    pub write: Option<
        unsafe extern "C" fn(stream: *const clap_ostream, buffer: *const c_void, size: u64) -> i64,
    >,
}

// --- Extensions ------------------------------------------------------------------------------

/// `CLAP_EXT_PARAMS`.
pub const CLAP_EXT_PARAMS: &CStr = c"clap.params";
/// `CLAP_EXT_STATE`.
pub const CLAP_EXT_STATE: &CStr = c"clap.state";
/// `CLAP_EXT_LATENCY`.
pub const CLAP_EXT_LATENCY: &CStr = c"clap.latency";
/// `CLAP_EXT_TAIL`.
pub const CLAP_EXT_TAIL: &CStr = c"clap.tail";
/// `CLAP_EXT_AUDIO_PORTS`.
pub const CLAP_EXT_AUDIO_PORTS: &CStr = c"clap.audio-ports";
/// `CLAP_EXT_NOTE_PORTS` (effects only here: the host doesn't provide note ports).
pub const CLAP_EXT_NOTE_PORTS: &CStr = c"clap.note-ports";
/// `CLAP_EXT_LOG`.
pub const CLAP_EXT_LOG: &CStr = c"clap.log";
/// `CLAP_EXT_THREAD_CHECK`.
pub const CLAP_EXT_THREAD_CHECK: &CStr = c"clap.thread-check";

/// `clap_param_info_flags`.
pub type clap_param_info_flags = u32;
/// Discrete values.
pub const CLAP_PARAM_IS_STEPPED: u32 = 1 << 0;
/// Periodic (angle).
pub const CLAP_PARAM_IS_PERIODIC: u32 = 1 << 1;
/// Not shown to the user.
pub const CLAP_PARAM_IS_HIDDEN: u32 = 1 << 2;
/// The host can't change it.
pub const CLAP_PARAM_IS_READONLY: u32 = 1 << 3;
/// The plugin's own bypass.
pub const CLAP_PARAM_IS_BYPASS: u32 = 1 << 4;
/// The host may automate it.
pub const CLAP_PARAM_IS_AUTOMATABLE: u32 = 1 << 5;
/// Modulatable.
pub const CLAP_PARAM_IS_MODULATABLE: u32 = 1 << 10;
/// Changes require processing.
pub const CLAP_PARAM_REQUIRES_PROCESS: u32 = 1 << 15;
/// An enumeration (implies stepped).
pub const CLAP_PARAM_IS_ENUM: u32 = 1 << 16;

/// `clap_param_info_t`.
#[repr(C)]
pub struct clap_param_info {
    /// Stable id.
    pub id: clap_id,
    /// Flags.
    pub flags: clap_param_info_flags,
    /// Plugin-owned pointer the host passes back in events (or null).
    pub cookie: *mut c_void,
    /// Display name, NUL-terminated.
    pub name: [c_char; CLAP_NAME_SIZE],
    /// Module path ("Oscillators/Wavetable 1"), NUL-terminated.
    pub module: [c_char; CLAP_PATH_SIZE],
    /// Minimum plain value.
    pub min_value: f64,
    /// Maximum plain value.
    pub max_value: f64,
    /// Default plain value.
    pub default_value: f64,
}

/// `clap_plugin_params_t`.
#[repr(C)]
pub struct clap_plugin_params {
    /// \[main\] Number of parameters.
    pub count: Option<unsafe extern "C" fn(plugin: *const clap_plugin) -> u32>,
    /// \[main\] Info of parameter `param_index`.
    pub get_info: Option<
        unsafe extern "C" fn(
            plugin: *const clap_plugin,
            param_index: u32,
            param_info: *mut clap_param_info,
        ) -> bool,
    >,
    /// \[main\] Current plain value.
    pub get_value: Option<
        unsafe extern "C" fn(
            plugin: *const clap_plugin,
            param_id: clap_id,
            out_value: *mut f64,
        ) -> bool,
    >,
    /// \[main\] Formats `value`.
    pub value_to_text: Option<
        unsafe extern "C" fn(
            plugin: *const clap_plugin,
            param_id: clap_id,
            value: f64,
            out_buffer: *mut c_char,
            out_buffer_capacity: u32,
        ) -> bool,
    >,
    /// \[main\] Parses text.
    pub text_to_value: Option<
        unsafe extern "C" fn(
            plugin: *const clap_plugin,
            param_id: clap_id,
            param_value_text: *const c_char,
            out_value: *mut f64,
        ) -> bool,
    >,
    /// \[active ? audio : main\] Applies events without processing audio.
    pub flush: Option<
        unsafe extern "C" fn(
            plugin: *const clap_plugin,
            in_: *const clap_input_events,
            out: *const clap_output_events,
        ),
    >,
}

/// `clap_host_params_t`.
#[repr(C)]
pub struct clap_host_params {
    /// \[main\] The parameter list or values changed.
    pub rescan: Option<unsafe extern "C" fn(host: *const clap_host, flags: u32)>,
    /// \[main\] Clears references to a parameter.
    pub clear: Option<unsafe extern "C" fn(host: *const clap_host, param_id: clap_id, flags: u32)>,
    /// \[thread-safe, !audio\] Asks for a `flush` or `process`.
    pub request_flush: Option<unsafe extern "C" fn(host: *const clap_host)>,
}

/// `clap_plugin_state_t`.
#[repr(C)]
pub struct clap_plugin_state {
    /// \[main\] Writes the state.
    pub save: Option<
        unsafe extern "C" fn(plugin: *const clap_plugin, stream: *const clap_ostream) -> bool,
    >,
    /// \[main\] Reads a state.
    pub load: Option<
        unsafe extern "C" fn(plugin: *const clap_plugin, stream: *const clap_istream) -> bool,
    >,
}

/// `clap_host_state_t`.
#[repr(C)]
pub struct clap_host_state {
    /// \[main\] The state changed (not a parameter).
    pub mark_dirty: Option<unsafe extern "C" fn(host: *const clap_host)>,
}

/// `clap_plugin_latency_t`.
#[repr(C)]
pub struct clap_plugin_latency {
    /// \[main & (being-activated | active)\] Latency in samples.
    pub get: Option<unsafe extern "C" fn(plugin: *const clap_plugin) -> u32>,
}

/// `clap_host_latency_t`.
#[repr(C)]
pub struct clap_host_latency {
    /// \[main & being-activated\] The latency changed.
    pub changed: Option<unsafe extern "C" fn(host: *const clap_host)>,
}

/// `clap_plugin_tail_t`.
#[repr(C)]
pub struct clap_plugin_tail {
    /// \[main, audio\] Tail in samples; ≥ `i32::MAX` = infinite.
    pub get: Option<unsafe extern "C" fn(plugin: *const clap_plugin) -> u32>,
}

/// PowerVoice's vendor extension (ADR-006 §2, T-805): a packaged module's
/// `vox_module_api::ModuleInfo` as JSON. Same id as `vox_module_api::MODULE_INFO_EXTENSION_ID`.
pub const POWERVOICE_EXT_MODULE_INFO: &CStr = c"org.powervoice.module-info/1";

/// The `org.powervoice.module-info/1` vtable (not part of CLAP; PowerVoice-defined, frozen for
/// `/1`).
#[repr(C)]
pub struct clap_plugin_module_info {
    /// \[main\] Writes the module info (UTF-8 JSON) to `stream`; `false` on failure.
    pub get_info: Option<
        unsafe extern "C" fn(plugin: *const clap_plugin, stream: *const clap_ostream) -> bool,
    >,
}

/// `clap_host_tail_t`.
#[repr(C)]
pub struct clap_host_tail {
    /// \[audio\] The tail changed.
    pub changed: Option<unsafe extern "C" fn(host: *const clap_host)>,
}

/// `CLAP_PORT_MONO`.
pub const CLAP_PORT_MONO: &CStr = c"mono";
/// `CLAP_PORT_STEREO`.
pub const CLAP_PORT_STEREO: &CStr = c"stereo";
/// `CLAP_AUDIO_PORT_IS_MAIN`.
pub const CLAP_AUDIO_PORT_IS_MAIN: u32 = 1 << 0;

/// `clap_audio_port_info_t`.
#[repr(C)]
pub struct clap_audio_port_info {
    /// Stable id.
    pub id: clap_id,
    /// Display name.
    pub name: [c_char; CLAP_NAME_SIZE],
    /// `CLAP_AUDIO_PORT_IS_MAIN`, …
    pub flags: u32,
    /// Channels.
    pub channel_count: u32,
    /// `CLAP_PORT_MONO`, `CLAP_PORT_STEREO` or null.
    pub port_type: *const c_char,
    /// In-place pair port id, or `CLAP_INVALID_ID`.
    pub in_place_pair: clap_id,
}

/// `clap_plugin_audio_ports_t`.
#[repr(C)]
pub struct clap_plugin_audio_ports {
    /// \[main\] Number of input or output ports.
    pub count: Option<unsafe extern "C" fn(plugin: *const clap_plugin, is_input: bool) -> u32>,
    /// \[main\] Port `index`.
    pub get: Option<
        unsafe extern "C" fn(
            plugin: *const clap_plugin,
            index: u32,
            is_input: bool,
            info: *mut clap_audio_port_info,
        ) -> bool,
    >,
}

/// `clap_host_audio_ports_t`.
#[repr(C)]
pub struct clap_host_audio_ports {
    /// \[main\] Whether the host supports a rescan flag.
    pub is_rescan_flag_supported:
        Option<unsafe extern "C" fn(host: *const clap_host, flag: u32) -> bool>,
    /// \[main\] The ports changed.
    pub rescan: Option<unsafe extern "C" fn(host: *const clap_host, flags: u32)>,
}

/// `clap_log_severity`.
pub type clap_log_severity = i32;
/// Debug.
pub const CLAP_LOG_DEBUG: clap_log_severity = 0;
/// Info.
pub const CLAP_LOG_INFO: clap_log_severity = 1;
/// Warning.
pub const CLAP_LOG_WARNING: clap_log_severity = 2;
/// Error.
pub const CLAP_LOG_ERROR: clap_log_severity = 3;
/// Fatal.
pub const CLAP_LOG_FATAL: clap_log_severity = 4;
/// The host misbehaved.
pub const CLAP_LOG_HOST_MISBEHAVING: clap_log_severity = 5;
/// The plugin misbehaved.
pub const CLAP_LOG_PLUGIN_MISBEHAVING: clap_log_severity = 6;

/// `clap_host_log_t`.
#[repr(C)]
pub struct clap_host_log {
    /// \[thread-safe\] Logs a message.
    pub log: Option<
        unsafe extern "C" fn(
            host: *const clap_host,
            severity: clap_log_severity,
            msg: *const c_char,
        ),
    >,
}

/// `clap_host_thread_check_t`.
#[repr(C)]
pub struct clap_host_thread_check {
    /// \[thread-safe\] The caller is the main thread.
    pub is_main_thread: Option<unsafe extern "C" fn(host: *const clap_host) -> bool>,
    /// \[thread-safe\] The caller is the audio thread.
    pub is_audio_thread: Option<unsafe extern "C" fn(host: *const clap_host) -> bool>,
}

// --- GUI, timers, file descriptors (T-901) ------------------------------------------------

/// `CLAP_EXT_GUI`.
pub const CLAP_EXT_GUI: &CStr = c"clap.gui";
/// `CLAP_EXT_TIMER_SUPPORT`.
pub const CLAP_EXT_TIMER_SUPPORT: &CStr = c"clap.timer-support";
/// `CLAP_EXT_POSIX_FD_SUPPORT`.
pub const CLAP_EXT_POSIX_FD_SUPPORT: &CStr = c"clap.posix-fd-support";

/// `CLAP_WINDOW_API_X11`: an X11 window id.
pub const CLAP_WINDOW_API_X11: &CStr = c"x11";
/// `CLAP_WINDOW_API_WIN32`: an `HWND`.
pub const CLAP_WINDOW_API_WIN32: &CStr = c"win32";
/// `CLAP_WINDOW_API_COCOA`: an `NSView*`.
pub const CLAP_WINDOW_API_COCOA: &CStr = c"cocoa";
/// `CLAP_WINDOW_API_WAYLAND` (floating only; not offered by PowerVoice).
pub const CLAP_WINDOW_API_WAYLAND: &CStr = c"wayland";

/// The handle of a `clap_window_t` (the C union).
#[repr(C)]
#[derive(Clone, Copy)]
pub union clap_window_handle {
    /// `clap_nsview`.
    pub cocoa: *mut c_void,
    /// `clap_xwnd` (`unsigned long`).
    pub x11: std::ffi::c_ulong,
    /// `clap_hwnd`.
    pub win32: *mut c_void,
    /// Any pointer.
    pub ptr: *mut c_void,
}

/// `clap_window_t`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct clap_window {
    /// One of the `CLAP_WINDOW_API_*` strings.
    pub api: *const c_char,
    /// The native handle.
    pub handle: clap_window_handle,
}

/// `clap_gui_resize_hints_t`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct clap_gui_resize_hints {
    /// Can resize horizontally.
    pub can_resize_horizontally: bool,
    /// Can resize vertically.
    pub can_resize_vertically: bool,
    /// Keeps an aspect ratio.
    pub preserve_aspect_ratio: bool,
    /// Aspect ratio width.
    pub aspect_ratio_width: u32,
    /// Aspect ratio height.
    pub aspect_ratio_height: u32,
}

/// `clap_plugin_gui_t` (every call is \[main\]).
#[repr(C)]
pub struct clap_plugin_gui {
    /// The API (and floating/embedded mode) is supported.
    pub is_api_supported: Option<
        unsafe extern "C" fn(
            plugin: *const clap_plugin,
            api: *const c_char,
            is_floating: bool,
        ) -> bool,
    >,
    /// The plugin's preferred API.
    pub get_preferred_api: Option<
        unsafe extern "C" fn(
            plugin: *const clap_plugin,
            api: *mut *const c_char,
            is_floating: *mut bool,
        ) -> bool,
    >,
    /// Creates the GUI.
    pub create: Option<
        unsafe extern "C" fn(
            plugin: *const clap_plugin,
            api: *const c_char,
            is_floating: bool,
        ) -> bool,
    >,
    /// Destroys it.
    pub destroy: Option<unsafe extern "C" fn(plugin: *const clap_plugin)>,
    /// Content scale.
    pub set_scale: Option<unsafe extern "C" fn(plugin: *const clap_plugin, scale: f64) -> bool>,
    /// Current size (embedded).
    pub get_size: Option<
        unsafe extern "C" fn(plugin: *const clap_plugin, width: *mut u32, height: *mut u32) -> bool,
    >,
    /// The GUI can be resized by the host.
    pub can_resize: Option<unsafe extern "C" fn(plugin: *const clap_plugin) -> bool>,
    /// Resize constraints.
    pub get_resize_hints: Option<
        unsafe extern "C" fn(plugin: *const clap_plugin, hints: *mut clap_gui_resize_hints) -> bool,
    >,
    /// Rounds a size to one the plugin accepts.
    pub adjust_size: Option<
        unsafe extern "C" fn(plugin: *const clap_plugin, width: *mut u32, height: *mut u32) -> bool,
    >,
    /// Sets the size (embedded).
    pub set_size:
        Option<unsafe extern "C" fn(plugin: *const clap_plugin, width: u32, height: u32) -> bool>,
    /// Embeds into `window`.
    pub set_parent: Option<
        unsafe extern "C" fn(plugin: *const clap_plugin, window: *const clap_window) -> bool,
    >,
    /// Floating: stays above `window`.
    pub set_transient: Option<
        unsafe extern "C" fn(plugin: *const clap_plugin, window: *const clap_window) -> bool,
    >,
    /// Floating: the window title.
    pub suggest_title:
        Option<unsafe extern "C" fn(plugin: *const clap_plugin, title: *const c_char)>,
    /// Shows the window.
    pub show: Option<unsafe extern "C" fn(plugin: *const clap_plugin) -> bool>,
    /// Hides it.
    pub hide: Option<unsafe extern "C" fn(plugin: *const clap_plugin) -> bool>,
}

/// `clap_host_gui_t`.
#[repr(C)]
pub struct clap_host_gui {
    /// \[thread-safe & !floating\] The resize hints changed.
    pub resize_hints_changed: Option<unsafe extern "C" fn(host: *const clap_host)>,
    /// \[thread-safe & !floating\] Asks the host to resize the parent window.
    pub request_resize:
        Option<unsafe extern "C" fn(host: *const clap_host, width: u32, height: u32) -> bool>,
    /// \[thread-safe\] Asks the host to show the window.
    pub request_show: Option<unsafe extern "C" fn(host: *const clap_host) -> bool>,
    /// \[thread-safe\] Asks the host to hide the window.
    pub request_hide: Option<unsafe extern "C" fn(host: *const clap_host) -> bool>,
    /// \[thread-safe\] The floating window was closed (or the connection to the GUI lost).
    pub closed: Option<unsafe extern "C" fn(host: *const clap_host, was_destroyed: bool)>,
}

/// `clap_plugin_timer_support_t`.
#[repr(C)]
pub struct clap_plugin_timer_support {
    /// \[main\] Timer `timer_id` fired.
    pub on_timer: Option<unsafe extern "C" fn(plugin: *const clap_plugin, timer_id: clap_id)>,
}

/// `clap_host_timer_support_t`.
#[repr(C)]
pub struct clap_host_timer_support {
    /// \[main\] Registers a periodic timer.
    pub register_timer: Option<
        unsafe extern "C" fn(
            host: *const clap_host,
            period_ms: u32,
            timer_id: *mut clap_id,
        ) -> bool,
    >,
    /// \[main\] Unregisters it.
    pub unregister_timer:
        Option<unsafe extern "C" fn(host: *const clap_host, timer_id: clap_id) -> bool>,
}

/// `clap_posix_fd_flags_t`: readable.
pub const CLAP_POSIX_FD_READ: u32 = 1 << 0;
/// Writable.
pub const CLAP_POSIX_FD_WRITE: u32 = 1 << 1;
/// Error.
pub const CLAP_POSIX_FD_ERROR: u32 = 1 << 2;

/// `clap_plugin_posix_fd_support_t`.
#[repr(C)]
pub struct clap_plugin_posix_fd_support {
    /// \[main\] `fd` is ready (`flags`: `CLAP_POSIX_FD_*`).
    pub on_fd: Option<unsafe extern "C" fn(plugin: *const clap_plugin, fd: c_int, flags: u32)>,
}

/// `clap_host_posix_fd_support_t`.
#[repr(C)]
pub struct clap_host_posix_fd_support {
    /// \[main\] Watches `fd`.
    pub register_fd:
        Option<unsafe extern "C" fn(host: *const clap_host, fd: c_int, flags: u32) -> bool>,
    /// \[main\] Changes what is watched.
    pub modify_fd:
        Option<unsafe extern "C" fn(host: *const clap_host, fd: c_int, flags: u32) -> bool>,
    /// \[main\] Stops watching.
    pub unregister_fd: Option<unsafe extern "C" fn(host: *const clap_host, fd: c_int) -> bool>,
}

/// A `*const c_char` that is `Sync`, for `static` descriptor tables pointing at string literals
/// (a raw pointer isn't `Sync` on its own).
#[repr(transparent)]
#[derive(Clone, Copy, Debug)]
pub struct StaticStr(pub *const c_char);

// SAFETY: only ever built from `&'static CStr` literals (immutable, 'static data).
unsafe impl Sync for StaticStr {}

impl StaticStr {
    /// From a string literal.
    pub const fn new(s: &'static CStr) -> Self {
        Self(s.as_ptr())
    }

    /// The null terminator of a feature list.
    pub const NULL: Self = Self(std::ptr::null());
}

/// Copies a NUL-terminated `c_char` array (e.g. [`clap_param_info::name`]) into a `String`
/// (lossy UTF-8; stops at the first NUL or the end of the array).
pub fn fixed_str(chars: &[c_char]) -> String {
    let bytes: Vec<u8> = chars
        .iter()
        .take_while(|&&c| c != 0)
        .map(|&c| c as u8)
        .collect();
    String::from_utf8_lossy(&bytes).into_owned()
}

/// Writes `s` into a fixed `c_char` array, truncating and always NUL-terminating.
pub fn write_fixed_str(dst: &mut [c_char], s: &str) {
    let Some(last) = dst.len().checked_sub(1) else {
        return;
    };
    let n = s.len().min(last);
    for (d, &b) in dst.iter_mut().zip(&s.as_bytes()[..n]) {
        *d = b as c_char;
    }
    dst[n] = 0;
}

/// A C string pointer as `Option<String>` (null → `None`; lossy UTF-8).
///
/// # Safety
/// `p` is null or points to a NUL-terminated string valid for the call.
pub unsafe fn opt_str(p: *const c_char) -> Option<String> {
    if p.is_null() {
        return None;
    }
    // SAFETY: non-null and NUL-terminated per the caller's contract.
    Some(unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::{offset_of, size_of};

    #[test]
    #[cfg(target_pointer_width = "64")]
    fn layouts_match_the_c_headers_on_64_bit() {
        assert_eq!(size_of::<clap_version>(), 12);
        assert_eq!(size_of::<clap_plugin_entry>(), 40);
        assert_eq!(size_of::<clap_plugin_factory>(), 24);
        assert_eq!(size_of::<clap_plugin_descriptor>(), 88);
        assert_eq!(size_of::<clap_plugin>(), 96);
        assert_eq!(size_of::<clap_host>(), 88);
        assert_eq!(offset_of!(clap_host, get_extension), 56);
        assert_eq!(size_of::<clap_audio_buffer>(), 32);
        assert_eq!(size_of::<clap_process>(), 64);
        assert_eq!(size_of::<clap_event_header>(), 16);
        assert_eq!(size_of::<clap_event_param_value>(), 56);
        assert_eq!(offset_of!(clap_event_param_value, cookie), 24);
        assert_eq!(offset_of!(clap_event_param_value, value), 48);
        assert_eq!(size_of::<clap_event_param_gesture>(), 20);
        assert_eq!(size_of::<clap_param_info>(), 1320);
        assert_eq!(offset_of!(clap_param_info, min_value), 1296);
        assert_eq!(size_of::<clap_audio_port_info>(), 288);
        assert_eq!(offset_of!(clap_audio_port_info, port_type), 272);
        assert_eq!(size_of::<clap_plugin_params>(), 48);
        assert_eq!(size_of::<clap_plugin_module_info>(), 8);
        // T-901
        assert_eq!(size_of::<clap_window>(), 16);
        assert_eq!(offset_of!(clap_window, handle), 8);
        assert_eq!(size_of::<clap_gui_resize_hints>(), 12);
        assert_eq!(offset_of!(clap_gui_resize_hints, aspect_ratio_width), 4);
        assert_eq!(size_of::<clap_plugin_gui>(), 15 * 8);
        assert_eq!(offset_of!(clap_plugin_gui, set_parent), 10 * 8);
        assert_eq!(size_of::<clap_host_gui>(), 5 * 8);
        assert_eq!(size_of::<clap_plugin_timer_support>(), 8);
        assert_eq!(size_of::<clap_host_timer_support>(), 16);
        assert_eq!(size_of::<clap_plugin_posix_fd_support>(), 8);
        assert_eq!(size_of::<clap_host_posix_fd_support>(), 24);
    }

    #[test]
    fn fixed_strings_round_trip_and_truncate() {
        let mut buf = [0 as c_char; 8];
        write_fixed_str(&mut buf, "gain");
        assert_eq!(fixed_str(&buf), "gain");
        write_fixed_str(&mut buf, "a much longer name");
        assert_eq!(fixed_str(&buf), "a much ");
        assert!(clap_version_is_compatible(CLAP_VERSION));
        assert!(!clap_version_is_compatible(clap_version {
            major: 0,
            minor: 9,
            revision: 0
        }));
    }
}

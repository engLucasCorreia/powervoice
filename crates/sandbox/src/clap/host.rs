//! The `clap_host` PowerVoice presents to a plugin, and the host extensions it answers.

use std::cell::Cell;
use std::ffi::{CStr, c_char, c_void};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, ThreadId};

use vox_clap_abi::*;

thread_local! {
    /// Set on the sandbox audio thread (`clap_host_thread_check.is_audio_thread`).
    static AUDIO_THREAD: Cell<bool> = const { Cell::new(false) };
}

/// Marks the calling thread as the audio thread (idempotent, no allocation).
pub(crate) fn mark_audio_thread() {
    AUDIO_THREAD.with(|a| a.set(true));
}

/// What the plugin asked the host for, acted on by the sandbox's main loop or audio thread.
pub(crate) struct HostState {
    main_thread: ThreadId,
    /// `request_restart` (forwarded to the editor as `EventKind::RESTART_REQUEST`).
    pub(crate) restart_requested: AtomicBool,
    /// `request_callback` (answered with `on_main_thread` by the idle loop).
    pub(crate) callback_requested: AtomicBool,
    /// `clap_host_params.request_flush` (answered with a `flush` while inactive).
    pub(crate) flush_requested: AtomicBool,
    /// Plugin name, for log lines.
    name: String,
}

/// The `clap_host` and its state at a stable address (the plugin keeps the pointer).
#[repr(C)]
pub(crate) struct HostBox {
    pub(crate) clap: clap_host,
    pub(crate) state: HostState,
}

impl HostBox {
    /// A host for plugin `name`, created on (and declaring) the main thread.
    pub(crate) fn new(name: &str) -> Box<Self> {
        let mut b = Box::new(Self {
            clap: clap_host {
                clap_version: CLAP_VERSION,
                host_data: std::ptr::null_mut(),
                name: c"PowerVoice".as_ptr(),
                vendor: c"PowerVoice".as_ptr(),
                url: c"https://powervoice.app".as_ptr(),
                version: c"0.1.0".as_ptr(),
                get_extension: Some(host_get_extension),
                request_restart: Some(host_request_restart),
                request_process: Some(host_request_process),
                request_callback: Some(host_request_callback),
            },
            state: HostState {
                main_thread: thread::current().id(),
                restart_requested: AtomicBool::new(false),
                callback_requested: AtomicBool::new(false),
                flush_requested: AtomicBool::new(false),
                name: name.to_owned(),
            },
        });
        b.clap.host_data = (&raw mut b.state).cast();
        b
    }
}

/// # Safety
/// `host` is null or a `clap_host` of a live [`HostBox`].
unsafe fn state<'a>(host: *const clap_host) -> Option<&'a HostState> {
    if host.is_null() {
        return None;
    }
    // SAFETY: caller's contract; `host_data` points at the box's `state`.
    let data = unsafe { (*host).host_data } as *const HostState;
    // SAFETY: non-null pointer into a live box.
    (!data.is_null()).then(|| unsafe { &*data })
}

unsafe extern "C" fn host_get_extension(
    _host: *const clap_host,
    id: *const c_char,
) -> *const c_void {
    if id.is_null() {
        return std::ptr::null();
    }
    // SAFETY: NUL-terminated per CLAP.
    let id = unsafe { CStr::from_ptr(id) };
    if id == CLAP_EXT_LOG {
        (&raw const HOST_LOG).cast()
    } else if id == CLAP_EXT_THREAD_CHECK {
        (&raw const HOST_THREAD_CHECK).cast()
    } else if id == CLAP_EXT_PARAMS {
        (&raw const HOST_PARAMS).cast()
    } else if id == CLAP_EXT_LATENCY {
        (&raw const HOST_LATENCY).cast()
    } else if id == CLAP_EXT_TAIL {
        (&raw const HOST_TAIL).cast()
    } else if id == CLAP_EXT_STATE {
        (&raw const HOST_STATE).cast()
    } else if id == CLAP_EXT_AUDIO_PORTS {
        (&raw const HOST_AUDIO_PORTS).cast()
    } else {
        std::ptr::null()
    }
}

unsafe extern "C" fn host_request_restart(host: *const clap_host) {
    // SAFETY: the plugin passes the host we gave it.
    if let Some(s) = unsafe { state(host) } {
        s.restart_requested.store(true, Ordering::Release);
    }
}

/// The sandbox always processes while active: nothing to do.
unsafe extern "C" fn host_request_process(_host: *const clap_host) {}

unsafe extern "C" fn host_request_callback(host: *const clap_host) {
    // SAFETY: the plugin passes the host we gave it.
    if let Some(s) = unsafe { state(host) } {
        s.callback_requested.store(true, Ordering::Release);
    }
}

static HOST_LOG: clap_host_log = clap_host_log {
    log: Some(host_log),
};

unsafe extern "C" fn host_log(
    host: *const clap_host,
    severity: clap_log_severity,
    msg: *const c_char,
) {
    let level = match severity {
        CLAP_LOG_DEBUG => "debug",
        CLAP_LOG_INFO => "info",
        CLAP_LOG_WARNING => "warning",
        CLAP_LOG_ERROR => "error",
        CLAP_LOG_FATAL => "fatal",
        CLAP_LOG_HOST_MISBEHAVING => "host misbehaving",
        CLAP_LOG_PLUGIN_MISBEHAVING => "plugin misbehaving",
        _ => "log",
    };
    // SAFETY: the plugin's host pointer and a NUL-terminated message (or null).
    let (name, text) = unsafe {
        (
            state(host).map_or("plugin", |s| s.name.as_str()),
            opt_str(msg).unwrap_or_default(),
        )
    };
    // The sandbox's log is its stderr (the editor inherits it; ADR-008 Amendment 2).
    eprintln!("powervoice-sandbox: {name} [{level}] {text}");
}

static HOST_THREAD_CHECK: clap_host_thread_check = clap_host_thread_check {
    is_main_thread: Some(host_is_main_thread),
    is_audio_thread: Some(host_is_audio_thread),
};

unsafe extern "C" fn host_is_main_thread(host: *const clap_host) -> bool {
    // SAFETY: the plugin passes the host we gave it.
    unsafe { state(host) }.is_some_and(|s| thread::current().id() == s.main_thread)
}

unsafe extern "C" fn host_is_audio_thread(_host: *const clap_host) -> bool {
    AUDIO_THREAD.with(Cell::get)
}

static HOST_PARAMS: clap_host_params = clap_host_params {
    rescan: Some(host_params_rescan),
    clear: Some(host_params_clear),
    request_flush: Some(host_params_request_flush),
};

/// Parameter list changes are picked up at the next instantiation (the schema is mirrored at
/// load); value changes arrive as output events or through `get_value` after a state load.
unsafe extern "C" fn host_params_rescan(_host: *const clap_host, _flags: u32) {}

unsafe extern "C" fn host_params_clear(_host: *const clap_host, _param_id: clap_id, _flags: u32) {}

unsafe extern "C" fn host_params_request_flush(host: *const clap_host) {
    // SAFETY: the plugin passes the host we gave it.
    if let Some(s) = unsafe { state(host) } {
        s.flush_requested.store(true, Ordering::Release);
    }
}

static HOST_LATENCY: clap_host_latency = clap_host_latency {
    changed: Some(host_noop),
};

static HOST_TAIL: clap_host_tail = clap_host_tail {
    changed: Some(host_noop),
};

/// `mark_dirty`: non-parameter state changes need a plugin GUI (T-901) — nothing to refresh.
static HOST_STATE: clap_host_state = clap_host_state {
    mark_dirty: Some(host_noop),
};

/// Latency is re-read at every activation; a tail change needs no action (the proxy's tail is
/// informational).
unsafe extern "C" fn host_noop(_host: *const clap_host) {}

static HOST_AUDIO_PORTS: clap_host_audio_ports = clap_host_audio_ports {
    is_rescan_flag_supported: Some(host_ports_flag_supported),
    rescan: Some(host_ports_rescan),
};

unsafe extern "C" fn host_ports_flag_supported(_host: *const clap_host, _flag: u32) -> bool {
    false
}

/// Port layouts are read at activation; a change while inactive is picked up then.
unsafe extern "C" fn host_ports_rescan(_host: *const clap_host, _flags: u32) {}

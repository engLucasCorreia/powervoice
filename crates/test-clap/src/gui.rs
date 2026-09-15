//! The test plugin's **trivial GUI** (T-901): embedded X11 only (`clap.gui`), a timer from the
//! host (`clap.timer-support`) and — when it gets a real parent window — a child X11 window whose
//! connection the host watches (`clap.posix-fd-support`).
//!
//! On the first timer tick after `show`, the GUI makes one edit, like a user turning a knob:
//! `gain_db` becomes [`GUI_EDIT_GAIN_DB`](crate::GUI_EDIT_GAIN_DB) (reported to the host as a
//! parameter change: in the next `process` while active, through `params.flush` otherwise) and
//! the GUI-only [`GUI_MARKER`](crate::GUI_MARKER) enters the state (`state.mark_dirty`). A copy
//! whose file name contains [`GUI_CRASH_MARKER`](crate::GUI_CRASH_MARKER) aborts on that tick
//! instead, one with [`GUI_HANG_MARKER`](crate::GUI_HANG_MARKER) never returns from it.
//!
//! With a parent window of 0 (the sandbox's headless window system) there is no child window:
//! the GUI still runs, without a display.

use std::ffi::{CStr, c_char};
use std::sync::atomic::Ordering;

use vox_clap_abi::*;

use crate::{GUI_CRASH_MARKER, GUI_HANG_MARKER, GUI_HEIGHT, GUI_TIMER_MS, GUI_WIDTH, Plugin, this};

/// The open GUI.
pub(crate) struct Gui {
    timer: Option<clap_id>,
    #[cfg(all(unix, not(target_os = "macos")))]
    child: Option<xchild::Child>,
    shown: bool,
    ticks: u32,
}

/// A host extension (null → `None`).
///
/// # Safety
/// `host` is the plugin's live host.
unsafe fn host_ext<'a, T>(host: *const clap_host, id: &CStr) -> Option<&'a T> {
    // SAFETY: caller's contract; `get_extension` is thread-safe.
    let p = unsafe {
        (*host)
            .get_extension
            .map_or(std::ptr::null(), |f| f(host, id.as_ptr()))
    } as *const T;
    // SAFETY: null or the host's vtable, valid while the host lives.
    unsafe { p.as_ref() }
}

pub(crate) static GUI: clap_plugin_gui = clap_plugin_gui {
    is_api_supported: Some(gui_is_api_supported),
    get_preferred_api: Some(gui_get_preferred_api),
    create: Some(gui_create),
    destroy: Some(gui_destroy),
    set_scale: Some(gui_set_scale),
    get_size: Some(gui_get_size),
    can_resize: Some(gui_can_resize),
    get_resize_hints: Some(gui_get_resize_hints),
    adjust_size: Some(gui_adjust_size),
    set_size: Some(gui_set_size),
    set_parent: Some(gui_set_parent),
    set_transient: Some(gui_set_transient),
    suggest_title: Some(gui_suggest_title),
    show: Some(gui_show),
    hide: Some(gui_hide),
};

pub(crate) static TIMER_SUPPORT: clap_plugin_timer_support = clap_plugin_timer_support {
    on_timer: Some(on_timer),
};

pub(crate) static POSIX_FD_SUPPORT: clap_plugin_posix_fd_support =
    clap_plugin_posix_fd_support { on_fd: Some(on_fd) };

fn is_x11(api: *const c_char) -> bool {
    // SAFETY: CLAP passes a NUL-terminated API name (or null).
    !api.is_null() && unsafe { CStr::from_ptr(api) } == CLAP_WINDOW_API_X11
}

unsafe extern "C" fn gui_is_api_supported(
    _: *const clap_plugin,
    api: *const c_char,
    is_floating: bool,
) -> bool {
    is_x11(api) && !is_floating
}

unsafe extern "C" fn gui_get_preferred_api(
    _: *const clap_plugin,
    api: *mut *const c_char,
    is_floating: *mut bool,
) -> bool {
    if api.is_null() || is_floating.is_null() {
        return false;
    }
    // SAFETY: the host's out-parameters, checked non-null.
    unsafe {
        *api = CLAP_WINDOW_API_X11.as_ptr();
        *is_floating = false;
    }
    true
}

unsafe extern "C" fn gui_create(
    p: *const clap_plugin,
    api: *const c_char,
    is_floating: bool,
) -> bool {
    if !is_x11(api) || is_floating {
        return false;
    }
    // SAFETY: a live plugin.
    let this = unsafe { this(p) };
    let mut gui = this.gui();
    if gui.is_some() {
        return false;
    }
    let mut timer = None;
    // SAFETY: the plugin's live host.
    if let Some(ts) =
        unsafe { host_ext::<clap_host_timer_support>(this.host, CLAP_EXT_TIMER_SUPPORT) }
        && let Some(register) = ts.register_timer
    {
        let mut id = CLAP_INVALID_ID;
        // SAFETY: main thread; writable id.
        if unsafe { register(this.host, GUI_TIMER_MS, &mut id) } {
            timer = Some(id);
        }
    }
    *gui = Some(Gui {
        timer,
        #[cfg(all(unix, not(target_os = "macos")))]
        child: None,
        shown: false,
        ticks: 0,
    });
    true
}

/// Unregisters the GUI's timer and descriptor and closes its child window.
pub(crate) fn teardown(this: &Plugin) {
    let Some(gui) = this.gui().take() else {
        return;
    };
    // SAFETY: the plugin's live host.
    unsafe {
        if let (Some(id), Some(ts)) = (
            gui.timer,
            host_ext::<clap_host_timer_support>(this.host, CLAP_EXT_TIMER_SUPPORT),
        ) && let Some(unregister) = ts.unregister_timer
        {
            unregister(this.host, id);
        }
        #[cfg(all(unix, not(target_os = "macos")))]
        if let (Some(child), Some(fds)) = (
            gui.child.as_ref(),
            host_ext::<clap_host_posix_fd_support>(this.host, CLAP_EXT_POSIX_FD_SUPPORT),
        ) && let Some(unregister) = fds.unregister_fd
        {
            unregister(this.host, child.fd());
        }
    }
}

unsafe extern "C" fn gui_destroy(p: *const clap_plugin) {
    // SAFETY: a live plugin.
    teardown(unsafe { this(p) });
}

unsafe extern "C" fn gui_set_scale(_: *const clap_plugin, _scale: f64) -> bool {
    true
}

unsafe extern "C" fn gui_get_size(
    _: *const clap_plugin,
    width: *mut u32,
    height: *mut u32,
) -> bool {
    if width.is_null() || height.is_null() {
        return false;
    }
    // SAFETY: the host's out-parameters, checked non-null.
    unsafe {
        *width = GUI_WIDTH;
        *height = GUI_HEIGHT;
    }
    true
}

unsafe extern "C" fn gui_can_resize(_: *const clap_plugin) -> bool {
    false
}

unsafe extern "C" fn gui_get_resize_hints(
    _: *const clap_plugin,
    _hints: *mut clap_gui_resize_hints,
) -> bool {
    false
}

unsafe extern "C" fn gui_adjust_size(_: *const clap_plugin, _w: *mut u32, _h: *mut u32) -> bool {
    false
}

unsafe extern "C" fn gui_set_size(_: *const clap_plugin, width: u32, height: u32) -> bool {
    (width, height) == (GUI_WIDTH, GUI_HEIGHT)
}

unsafe extern "C" fn gui_set_parent(p: *const clap_plugin, window: *const clap_window) -> bool {
    if window.is_null() {
        return false;
    }
    // SAFETY: a live plugin; the host's window for the call.
    let (this, window) = unsafe { (this(p), &*window) };
    if !is_x11(window.api) {
        return false;
    }
    // SAFETY: an X11 window: the union's `x11` member.
    let parent = unsafe { window.handle.x11 };
    let mut gui = this.gui();
    let Some(g) = gui.as_mut() else {
        return false;
    };
    #[cfg(all(unix, not(target_os = "macos")))]
    if parent != 0
        && let Some(child) = xchild::Child::new(parent)
    {
        // SAFETY: the plugin's live host.
        if let Some(fds) =
            unsafe { host_ext::<clap_host_posix_fd_support>(this.host, CLAP_EXT_POSIX_FD_SUPPORT) }
            && let Some(register) = fds.register_fd
        {
            // SAFETY: main thread.
            unsafe { register(this.host, child.fd(), CLAP_POSIX_FD_READ) };
        }
        g.child = Some(child);
    }
    let _ = (parent, g);
    true
}

unsafe extern "C" fn gui_set_transient(_: *const clap_plugin, _window: *const clap_window) -> bool {
    false
}

unsafe extern "C" fn gui_suggest_title(_: *const clap_plugin, _title: *const c_char) {}

unsafe extern "C" fn gui_show(p: *const clap_plugin) -> bool {
    // SAFETY: a live plugin.
    match unsafe { this(p) }.gui().as_mut() {
        Some(g) => {
            g.shown = true;
            true
        }
        None => false,
    }
}

unsafe extern "C" fn gui_hide(p: *const clap_plugin) -> bool {
    // SAFETY: a live plugin.
    match unsafe { this(p) }.gui().as_mut() {
        Some(g) => {
            g.shown = false;
            true
        }
        None => false,
    }
}

unsafe extern "C" fn on_timer(p: *const clap_plugin, timer_id: clap_id) {
    // SAFETY: a live plugin.
    let this = unsafe { this(p) };
    let first = match this.gui().as_mut() {
        Some(g) if g.shown && g.timer == Some(timer_id) => {
            g.ticks += 1;
            g.ticks == 1
        }
        _ => false,
    };
    if !first {
        return;
    }
    let file = crate::PLUGIN_FILE.get().map_or("", String::as_str);
    if file.contains(GUI_CRASH_MARKER) {
        crate::crash_without_core_dump();
    }
    if file.contains(GUI_HANG_MARKER) {
        loop {
            std::thread::sleep(std::time::Duration::from_secs(3600));
        }
    }
    this.gui_edit();
}

unsafe extern "C" fn on_fd(p: *const clap_plugin, _fd: i32, _flags: u32) {
    // SAFETY: a live plugin.
    let this = unsafe { this(p) };
    #[cfg(all(unix, not(target_os = "macos")))]
    if let Some(child) = this.gui().as_ref().and_then(|g| g.child.as_ref()) {
        child.drain();
    }
    let _ = this;
}

impl Plugin {
    /// The GUI's one edit (a knob turned, plus a GUI-only setting).
    fn gui_edit(&self) {
        let db = vox_modules::Gain::gain_param().clamp_quantize(crate::GUI_EDIT_GAIN_DB);
        self.gain_db.store(db.to_bits(), Ordering::Release);
        self.gui_marker.store(crate::GUI_MARKER, Ordering::Release);
        self.gui_edit_pending.store(true, Ordering::Release);
        // SAFETY: the plugin's live host; main thread (a timer callback).
        unsafe {
            if let Some(st) = host_ext::<clap_host_state>(self.host, CLAP_EXT_STATE)
                && let Some(mark_dirty) = st.mark_dirty
            {
                mark_dirty(self.host);
            }
            // Inactive: the host flushes (the next `process` reports it while active).
            if !self.active.load(Ordering::Acquire)
                && let Some(params) = host_ext::<clap_host_params>(self.host, CLAP_EXT_PARAMS)
                && let Some(request_flush) = params.request_flush
            {
                request_flush(self.host);
            }
        }
    }
}

/// A child X11 window of the host's window, through the system's libX11 loaded at run time (only
/// when a real parent window is given: the opt-in real-window test).
#[cfg(all(unix, not(target_os = "macos")))]
mod xchild {
    use std::ffi::{c_char, c_int, c_long, c_uint, c_ulong, c_void};
    use std::sync::OnceLock;

    type Display = c_void;

    struct Xlib {
        open: unsafe extern "C" fn(*const c_char) -> *mut Display,
        close: unsafe extern "C" fn(*mut Display) -> c_int,
        create: unsafe extern "C" fn(
            *mut Display,
            c_ulong,
            c_int,
            c_int,
            c_uint,
            c_uint,
            c_uint,
            c_ulong,
            c_ulong,
        ) -> c_ulong,
        map: unsafe extern "C" fn(*mut Display, c_ulong) -> c_int,
        destroy: unsafe extern "C" fn(*mut Display, c_ulong) -> c_int,
        flush: unsafe extern "C" fn(*mut Display) -> c_int,
        pending: unsafe extern "C" fn(*mut Display) -> c_int,
        next_event: unsafe extern "C" fn(*mut Display, *mut c_long) -> c_int,
        connection: unsafe extern "C" fn(*mut Display) -> c_int,
        _lib: libloading::Library,
    }

    fn xlib() -> Option<&'static Xlib> {
        static X: OnceLock<Option<Xlib>> = OnceLock::new();
        X.get_or_init(|| {
            // SAFETY: the system's libX11; the symbols have Xlib's documented signatures and stay
            // valid while `_lib` is loaded.
            unsafe {
                let lib = libloading::Library::new("libX11.so.6").ok()?;
                macro_rules! sym {
                    ($n:literal) => {
                        *lib.get($n).ok()?
                    };
                }
                Some(Xlib {
                    open: sym!(b"XOpenDisplay\0"),
                    close: sym!(b"XCloseDisplay\0"),
                    create: sym!(b"XCreateSimpleWindow\0"),
                    map: sym!(b"XMapWindow\0"),
                    destroy: sym!(b"XDestroyWindow\0"),
                    flush: sym!(b"XFlush\0"),
                    pending: sym!(b"XPending\0"),
                    next_event: sym!(b"XNextEvent\0"),
                    connection: sym!(b"XConnectionNumber\0"),
                    _lib: lib,
                })
            }
        })
        .as_ref()
    }

    pub(crate) struct Child {
        x: &'static Xlib,
        dpy: *mut Display,
        window: c_ulong,
    }

    impl Child {
        /// A light grey child window filling `parent`.
        pub(crate) fn new(parent: c_ulong) -> Option<Self> {
            let x = xlib()?;
            // SAFETY: Xlib calls on the plugin's main thread with our own display.
            unsafe {
                let dpy = (x.open)(std::ptr::null());
                if dpy.is_null() {
                    return None;
                }
                let window = (x.create)(
                    dpy,
                    parent,
                    0,
                    0,
                    crate::GUI_WIDTH,
                    crate::GUI_HEIGHT,
                    0,
                    0,
                    0x00d8_dce3,
                );
                (x.map)(dpy, window);
                (x.flush)(dpy);
                Some(Self { x, dpy, window })
            }
        }

        pub(crate) fn fd(&self) -> i32 {
            // SAFETY: our live display.
            unsafe { (self.x.connection)(self.dpy) }
        }

        /// Takes whatever the X server sent.
        pub(crate) fn drain(&self) {
            let mut ev = [0 as c_long; 24];
            // SAFETY: our live display; `ev` is a full `XEvent`.
            unsafe {
                while (self.x.pending)(self.dpy) > 0 {
                    (self.x.next_event)(self.dpy, ev.as_mut_ptr());
                }
            }
        }
    }

    impl Drop for Child {
        fn drop(&mut self) {
            // SAFETY: our window and display, released once.
            unsafe {
                (self.x.destroy)(self.dpy, self.window);
                (self.x.close)(self.dpy);
            }
        }
    }
}

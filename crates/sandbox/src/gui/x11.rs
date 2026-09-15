//! X11 top-level windows through the system's **libX11, loaded at run time** (T-901, ADR-008
//! Amendment 13) — no build-time dependency and nothing to install to build, like lilv
//! (Amendment 8). Without libX11, or without an X server (under Wayland that is XWayland), plugin
//! windows are unavailable and nothing else is affected.
//!
//! The window is a plain top-level window with the title, `WM_CLASS` "PowerVoice", a dialog
//! window type (tiling window managers float it), `WM_DELETE_WINDOW`, fixed size hints unless the
//! plugin is resizable, and `WM_TRANSIENT_FOR` the editor's window when the editor itself is an
//! X11 window. X errors are ignored rather than fatal (a stale transient-for id must not kill the
//! sandbox).

use std::ffi::{CString, c_char, c_int, c_long, c_uchar, c_uint, c_ulong, c_void};
use std::sync::OnceLock;

use super::{Backend, WindowEvent, WindowSpec};

type Display = c_void;
type Window = c_ulong;
type Atom = c_ulong;

/// `XA_ATOM`.
const XA_ATOM: Atom = 4;
/// `XA_CARDINAL`.
const XA_CARDINAL: Atom = 6;
/// `PropModeReplace`.
const PROP_MODE_REPLACE: c_int = 0;
/// `StructureNotifyMask`.
const STRUCTURE_NOTIFY_MASK: c_long = 1 << 17;
/// Event types.
const DESTROY_NOTIFY: c_int = 17;
const CONFIGURE_NOTIFY: c_int = 22;
const CLIENT_MESSAGE: c_int = 33;
/// `XSizeHints` flags.
const P_SIZE: c_long = 1 << 3;
const P_MIN_SIZE: c_long = 1 << 4;
const P_MAX_SIZE: c_long = 1 << 5;

#[repr(C)]
#[derive(Default)]
struct XSizeHints {
    flags: c_long,
    x: c_int,
    y: c_int,
    width: c_int,
    height: c_int,
    min_width: c_int,
    min_height: c_int,
    max_width: c_int,
    max_height: c_int,
    width_inc: c_int,
    height_inc: c_int,
    min_aspect: [c_int; 2],
    max_aspect: [c_int; 2],
    base_width: c_int,
    base_height: c_int,
    win_gravity: c_int,
}

#[repr(C)]
struct XClassHint {
    res_name: *mut c_char,
    res_class: *mut c_char,
}

/// `XEvent` (a union of at most 24 `long`s).
#[repr(C)]
struct XEvent {
    pad: [c_long; 24],
}

#[repr(C)]
struct XClientMessageEvent {
    type_: c_int,
    serial: c_ulong,
    send_event: c_int,
    display: *mut Display,
    window: Window,
    message_type: Atom,
    format: c_int,
    data: [c_long; 5],
}

#[repr(C)]
struct XConfigureEvent {
    type_: c_int,
    serial: c_ulong,
    send_event: c_int,
    display: *mut Display,
    event: Window,
    window: Window,
    x: c_int,
    y: c_int,
    width: c_int,
    height: c_int,
    border_width: c_int,
    above: Window,
    override_redirect: c_int,
}

#[repr(C)]
struct XDestroyWindowEvent {
    type_: c_int,
    serial: c_ulong,
    send_event: c_int,
    display: *mut Display,
    event: Window,
    window: Window,
}

type ErrorHandler = unsafe extern "C" fn(*mut Display, *mut c_void) -> c_int;

macro_rules! xlib_api {
    ($($field:ident : fn($($arg:ty),*) $(-> $ret:ty)?;)*) => {
        /// The resolved Xlib functions (field `x` is `Xx`).
        #[allow(non_snake_case)]
        struct Xlib {
            $($field: unsafe extern "C" fn($($arg),*) $(-> $ret)?,)*
            _lib: libloading::Library,
        }

        impl Xlib {
            #[allow(non_snake_case)]
            fn resolve(lib: libloading::Library) -> Result<Self, String> {
                $(
                    let name = concat!("X", stringify!($field), "\0");
                    // SAFETY: the symbol's type is Xlib's documented C signature (Xlib.h,
                    // Xutil.h); the pointer stays valid while `_lib` is loaded.
                    let $field = unsafe {
                        lib.get::<unsafe extern "C" fn($($arg),*) $(-> $ret)?>(name.as_bytes())
                    }
                    .map(|s| *s)
                    .map_err(|e| format!("libX11 is missing {} ({e})", &name[..name.len() - 1]))?;
                )*
                Ok(Self { $($field,)* _lib: lib })
            }
        }
    };
}

xlib_api! {
    InitThreads: fn() -> c_int;
    OpenDisplay: fn(*const c_char) -> *mut Display;
    CloseDisplay: fn(*mut Display) -> c_int;
    DefaultScreen: fn(*mut Display) -> c_int;
    RootWindow: fn(*mut Display, c_int) -> Window;
    BlackPixel: fn(*mut Display, c_int) -> c_ulong;
    CreateSimpleWindow: fn(*mut Display, Window, c_int, c_int, c_uint, c_uint, c_uint, c_ulong, c_ulong) -> Window;
    DestroyWindow: fn(*mut Display, Window) -> c_int;
    MapRaised: fn(*mut Display, Window) -> c_int;
    RaiseWindow: fn(*mut Display, Window) -> c_int;
    ResizeWindow: fn(*mut Display, Window, c_uint, c_uint) -> c_int;
    StoreName: fn(*mut Display, Window, *const c_char) -> c_int;
    InternAtom: fn(*mut Display, *const c_char, c_int) -> Atom;
    ChangeProperty: fn(*mut Display, Window, Atom, Atom, c_int, c_int, *const c_uchar, c_int) -> c_int;
    SetWMProtocols: fn(*mut Display, Window, *mut Atom, c_int) -> c_int;
    SetTransientForHint: fn(*mut Display, Window, Window) -> c_int;
    SetWMNormalHints: fn(*mut Display, Window, *mut XSizeHints);
    SetClassHint: fn(*mut Display, Window, *mut XClassHint) -> c_int;
    SelectInput: fn(*mut Display, Window, c_long) -> c_int;
    Pending: fn(*mut Display) -> c_int;
    NextEvent: fn(*mut Display, *mut XEvent) -> c_int;
    Flush: fn(*mut Display) -> c_int;
    Sync: fn(*mut Display, c_int) -> c_int;
    ConnectionNumber: fn(*mut Display) -> c_int;
    SetErrorHandler: fn(Option<ErrorHandler>) -> Option<ErrorHandler>;
}

/// libX11's soname (and the unversioned name some systems only have).
const LIBX11: [&str; 2] = ["libX11.so.6", "libX11.so"];

fn xlib() -> Result<&'static Xlib, String> {
    static X: OnceLock<Result<Xlib, String>> = OnceLock::new();
    X.get_or_init(|| {
        let mut last = String::new();
        for name in LIBX11 {
            // SAFETY: loading the system's libX11 runs its (trivial) initialisers only.
            match unsafe { libloading::Library::new(name) } {
                Ok(lib) => return Xlib::resolve(lib),
                Err(e) => last = e.to_string(),
            }
        }
        Err(format!(
            "plugin windows need the X11 library (libX11), which couldn't be loaded ({last})"
        ))
    })
    .as_ref()
    .map_err(Clone::clone)
}

/// X errors are reported, not fatal (Xlib's default handler exits the process).
unsafe extern "C" fn ignore_error(_display: *mut Display, _event: *mut c_void) -> c_int {
    0
}

pub(super) struct X11 {
    x: &'static Xlib,
    dpy: *mut Display,
    root: Window,
    black: c_ulong,
    wm_protocols: Atom,
    wm_delete: Atom,
    window: Option<Window>,
    size: (u32, u32),
}

impl X11 {
    /// Connects to the display `$DISPLAY` names.
    pub(super) fn connect() -> Result<Self, String> {
        let x = xlib()?;
        // SAFETY: Xlib calls on this (the main) thread; `XInitThreads` first, since plugin GUIs
        // may use Xlib from their own threads.
        unsafe {
            (x.InitThreads)();
            let dpy = (x.OpenDisplay)(std::ptr::null());
            if dpy.is_null() {
                return Err(
                    "plugin windows need an X11 display (or XWayland), and none is available"
                        .into(),
                );
            }
            (x.SetErrorHandler)(Some(ignore_error));
            let screen = (x.DefaultScreen)(dpy);
            let me = Self {
                x,
                dpy,
                root: (x.RootWindow)(dpy, screen),
                black: (x.BlackPixel)(dpy, screen),
                wm_protocols: (x.InternAtom)(dpy, c"WM_PROTOCOLS".as_ptr(), 0),
                wm_delete: (x.InternAtom)(dpy, c"WM_DELETE_WINDOW".as_ptr(), 0),
                window: None,
                size: (0, 0),
            };
            Ok(me)
        }
    }

    fn atom(&self, name: &std::ffi::CStr) -> Atom {
        // SAFETY: a live display; NUL-terminated name.
        unsafe { (self.x.InternAtom)(self.dpy, name.as_ptr(), 0) }
    }

    fn set_size_hints(&self, w: Window, width: u32, height: u32, resizable: bool) {
        let (cw, ch) = (clamp_dim(width), clamp_dim(height));
        let mut hints = XSizeHints {
            flags: P_SIZE,
            width: cw,
            height: ch,
            ..XSizeHints::default()
        };
        if !resizable {
            hints.flags |= P_MIN_SIZE | P_MAX_SIZE;
            (hints.min_width, hints.min_height) = (cw, ch);
            (hints.max_width, hints.max_height) = (cw, ch);
        }
        // SAFETY: a live display and window; `hints` outlives the call.
        unsafe { (self.x.SetWMNormalHints)(self.dpy, w, &mut hints) };
    }
}

fn clamp_dim(v: u32) -> c_int {
    c_int::try_from(v.clamp(1, 16_384)).unwrap_or(1)
}

impl Backend for X11 {
    fn create(&mut self, spec: &WindowSpec<'_>) -> Result<u64, String> {
        let x = self.x;
        let title = CString::new(spec.title.replace('\0', "")).unwrap_or_default();
        // SAFETY: Xlib calls on a live display from the main thread; every buffer outlives its
        // call.
        unsafe {
            let w = (x.CreateSimpleWindow)(
                self.dpy,
                self.root,
                0,
                0,
                clamp_dim(spec.width) as c_uint,
                clamp_dim(spec.height) as c_uint,
                0,
                self.black,
                self.black,
            );
            if w == 0 {
                return Err("couldn't create the plugin window".into());
            }
            (x.SelectInput)(self.dpy, w, STRUCTURE_NOTIFY_MASK);
            (x.StoreName)(self.dpy, w, title.as_ptr());
            let utf8 = self.atom(c"UTF8_STRING");
            let net_name = self.atom(c"_NET_WM_NAME");
            let bytes = title.as_bytes();
            (x.ChangeProperty)(
                self.dpy,
                w,
                net_name,
                utf8,
                8,
                PROP_MODE_REPLACE,
                bytes.as_ptr(),
                c_int::try_from(bytes.len()).unwrap_or(0),
            );
            // A dialog: tiling window managers (Hyprland included) float it.
            let window_type = self.atom(c"_NET_WM_WINDOW_TYPE");
            let dialog = self.atom(c"_NET_WM_WINDOW_TYPE_DIALOG");
            (x.ChangeProperty)(
                self.dpy,
                w,
                window_type,
                XA_ATOM,
                32,
                PROP_MODE_REPLACE,
                (&raw const dialog).cast(),
                1,
            );
            let pid = c_long::from(std::process::id() as i32);
            let net_pid = self.atom(c"_NET_WM_PID");
            (x.ChangeProperty)(
                self.dpy,
                w,
                net_pid,
                XA_CARDINAL,
                32,
                PROP_MODE_REPLACE,
                (&raw const pid).cast(),
                1,
            );
            let mut name = *b"powervoice-plugin\0";
            let mut class = *b"PowerVoice\0";
            let mut hint = XClassHint {
                res_name: name.as_mut_ptr().cast(),
                res_class: class.as_mut_ptr().cast(),
            };
            (x.SetClassHint)(self.dpy, w, &mut hint);
            let mut protocols = [self.wm_delete];
            (x.SetWMProtocols)(self.dpy, w, protocols.as_mut_ptr(), 1);
            if let Some(parent) = spec.transient_for.filter(|&p| p != 0) {
                (x.SetTransientForHint)(self.dpy, w, parent as Window);
            }
            self.set_size_hints(w, spec.width, spec.height, spec.resizable);
            // Errors (a stale transient-for id) surface here and are ignored.
            (x.Sync)(self.dpy, 0);
            self.window = Some(w);
            self.size = (spec.width, spec.height);
            // `Window` is a C `unsigned long`: 64-bit here, 32-bit on some targets.
            #[allow(clippy::useless_conversion)]
            let raw = u64::from(w);
            Ok(raw)
        }
    }

    fn show(&mut self) {
        if let Some(w) = self.window {
            // SAFETY: a live display and window.
            unsafe {
                (self.x.MapRaised)(self.dpy, w);
                (self.x.Flush)(self.dpy);
            }
        }
    }

    fn raise(&mut self) {
        if let Some(w) = self.window {
            // SAFETY: as above.
            unsafe {
                (self.x.RaiseWindow)(self.dpy, w);
                (self.x.Flush)(self.dpy);
            }
        }
    }

    fn resize(&mut self, width: u32, height: u32) {
        if let Some(w) = self.window {
            self.size = (width, height);
            // SAFETY: as above.
            unsafe {
                (self.x.ResizeWindow)(
                    self.dpy,
                    w,
                    clamp_dim(width) as c_uint,
                    clamp_dim(height) as c_uint,
                );
                (self.x.Flush)(self.dpy);
            }
        }
    }

    fn destroy(&mut self) {
        if let Some(w) = self.window.take() {
            // SAFETY: as above; the plugin detached its GUI first.
            unsafe {
                (self.x.DestroyWindow)(self.dpy, w);
                (self.x.Sync)(self.dpy, 0);
            }
        }
    }

    fn fd(&self) -> Option<i32> {
        // SAFETY: a live display.
        Some(unsafe { (self.x.ConnectionNumber)(self.dpy) })
    }

    fn pump(&mut self, out: &mut Vec<WindowEvent>) {
        let x = self.x;
        // SAFETY: a live display; `ev` is a full `XEvent`, read through the layout of the type
        // it reports.
        unsafe {
            while (x.Pending)(self.dpy) > 0 {
                let mut ev = XEvent { pad: [0; 24] };
                (x.NextEvent)(self.dpy, &mut ev);
                let type_ = *(ev.pad.as_ptr() as *const c_int);
                let base = ev.pad.as_ptr();
                match type_ {
                    CLIENT_MESSAGE => {
                        let e = &*(base as *const XClientMessageEvent);
                        if Some(e.window) == self.window
                            && e.message_type == self.wm_protocols
                            && e.data[0] as Atom == self.wm_delete
                        {
                            out.push(WindowEvent::CloseRequested);
                        }
                    }
                    CONFIGURE_NOTIFY => {
                        let e = &*(base as *const XConfigureEvent);
                        let size = (e.width.max(1) as u32, e.height.max(1) as u32);
                        if Some(e.window) == self.window && size != self.size {
                            self.size = size;
                            out.push(WindowEvent::Resized(size.0, size.1));
                        }
                    }
                    DESTROY_NOTIFY => {
                        let e = &*(base as *const XDestroyWindowEvent);
                        if Some(e.window) == self.window {
                            self.window = None;
                            out.push(WindowEvent::CloseRequested);
                        }
                    }
                    _ => {}
                }
            }
        }
    }
}

impl Drop for X11 {
    fn drop(&mut self) {
        self.destroy();
        // SAFETY: the display this struct opened, closed once.
        unsafe { (self.x.CloseDisplay)(self.dpy) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::{offset_of, size_of};

    #[test]
    #[cfg(target_pointer_width = "64")]
    fn event_layouts_match_xlib_on_64_bit() {
        assert_eq!(size_of::<XEvent>(), 192);
        assert_eq!(offset_of!(XClientMessageEvent, message_type), 40);
        assert_eq!(offset_of!(XClientMessageEvent, data), 56);
        assert_eq!(offset_of!(XConfigureEvent, width), 56);
        assert_eq!(offset_of!(XDestroyWindowEvent, window), 40);
        assert_eq!(size_of::<XSizeHints>(), 80);
    }
}

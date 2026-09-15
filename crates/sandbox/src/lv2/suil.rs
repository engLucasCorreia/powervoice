//! LV2 plugin UIs through **suil, loaded at run time** (T-901, ADR-008 Amendment 13), like lilv
//! (Amendment 8): no build-time dependency. suil instantiates the plugin's `ui:X11UI` (or a UI it
//! can wrap into one) as a child of the sandbox's window (`ui:parent`). Without suil — or without
//! a UI it can show — the plugin simply has no window.
//!
//! Features given to the UI: `ui:parent`, `ui:resize`, `urid:map`/`urid:unmap` (a table of the
//! UI's own: PowerVoice exchanges only control values with it, never atom messages) and
//! `ui:idleInterface` (called on the idle tick). Not given: `instance-access` / `data-access` —
//! a UI that requires them fails to open (graceful error).

use std::ffi::{CString, c_char, c_int, c_uint, c_void};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard, OnceLock, PoisonError};

use vox_lv2_abi::{LV2_Feature, LV2UI_Idle_Interface, LV2UI_Resize, uri};

use super::host::Host;
use super::lilv::UiChoice;

/// The message when suil can't be loaded.
pub(crate) const SUIL_MISSING: &str =
    "LV2 plugin windows need the suil library (libsuil-0), which isn't installed";

/// `LV2UI_INVALID_PORT_INDEX`.
const INVALID_PORT: u32 = u32::MAX;
/// [`UiCtl`]'s resize cell when there is no request.
const NO_RESIZE: u64 = u64::MAX;

type M = *mut c_void;
type WriteFn = unsafe extern "C" fn(M, u32, u32, u32, *const c_void);
type IndexFn = unsafe extern "C" fn(M, *const c_char) -> u32;
type SubscribeFn = unsafe extern "C" fn(M, u32, u32, *const *const LV2_Feature) -> u32;

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The resolved suil functions.
pub(crate) struct Suil {
    /// `suil_ui_supported` (lilv's `LilvUISupportedFunc`).
    pub(crate) ui_supported: unsafe extern "C" fn(*const c_char, *const c_char) -> c_uint,
    host_new: unsafe extern "C" fn(
        Option<WriteFn>,
        Option<IndexFn>,
        Option<SubscribeFn>,
        Option<SubscribeFn>,
    ) -> M,
    host_free: unsafe extern "C" fn(M),
    #[allow(clippy::type_complexity)]
    instance_new: unsafe extern "C" fn(
        M,
        M,
        *const c_char,
        *const c_char,
        *const c_char,
        *const c_char,
        *const c_char,
        *const c_char,
        *const *const LV2_Feature,
    ) -> M,
    instance_free: unsafe extern "C" fn(M),
    instance_get_handle: unsafe extern "C" fn(M) -> M,
    instance_port_event: unsafe extern "C" fn(M, u32, u32, u32, *const c_void),
    instance_extension_data: unsafe extern "C" fn(M, *const c_char) -> *const c_void,
    _lib: libloading::Library,
}

fn load() -> Result<Suil, String> {
    let mut last = String::from("no candidate");
    for name in ["libsuil-0.so.0", "libsuil-0.so"] {
        // SAFETY: the system's suil; its initialisers only run in this sandbox process.
        let lib = match unsafe { libloading::Library::new(name) } {
            Ok(l) => l,
            Err(e) => {
                last = e.to_string();
                continue;
            }
        };
        macro_rules! sym {
            ($n:literal) => {
                // SAFETY: suil.h's documented signature; valid while `_lib` is loaded.
                *unsafe { lib.get(concat!($n, "\0").as_bytes()) }
                    .map_err(|e| format!("{SUIL_MISSING}: {} is missing ({e})", $n))?
            };
        }
        return Ok(Suil {
            ui_supported: sym!("suil_ui_supported"),
            host_new: sym!("suil_host_new"),
            host_free: sym!("suil_host_free"),
            instance_new: sym!("suil_instance_new"),
            instance_free: sym!("suil_instance_free"),
            instance_get_handle: sym!("suil_instance_get_handle"),
            instance_port_event: sym!("suil_instance_port_event"),
            instance_extension_data: sym!("suil_instance_extension_data"),
            _lib: lib,
        });
    }
    Err(format!("{SUIL_MISSING} ({last})"))
}

/// The process's suil, loaded on first use.
pub(crate) fn api() -> Result<&'static Suil, String> {
    static SUIL: OnceLock<Result<Suil, String>> = OnceLock::new();
    SUIL.get_or_init(load).as_ref().map_err(Clone::clone)
}

/// What a UI tells the host (written by suil's callbacks, on the main thread).
pub(crate) struct UiCtl {
    /// `(port, value)` the UI wrote, oldest first.
    writes: Mutex<Vec<(u32, f32)>>,
    /// `ui:resize`: `(width << 32) | height` ([`NO_RESIZE`]: none).
    resize: AtomicU64,
    /// Port symbols → indices (`SuilPortIndexFunc`).
    symbols: Vec<(CString, u32)>,
}

impl UiCtl {
    pub(crate) fn new(symbols: Vec<(String, u32)>) -> Self {
        Self {
            writes: Mutex::new(Vec::new()),
            resize: AtomicU64::new(NO_RESIZE),
            symbols: symbols
                .into_iter()
                .filter_map(|(s, i)| CString::new(s).ok().map(|c| (c, i)))
                .collect(),
        }
    }

    /// The values the UI wrote since the last call.
    pub(crate) fn take_writes(&self) -> Vec<(u32, f32)> {
        std::mem::take(&mut *lock(&self.writes))
    }

    /// The size the UI asked for since the last call.
    pub(crate) fn take_resize(&self) -> Option<(u32, u32)> {
        let r = self.resize.swap(NO_RESIZE, Ordering::AcqRel);
        (r != NO_RESIZE).then_some(((r >> 32) as u32, r as u32))
    }
}

/// `SuilPortWriteFunc`: a control value (protocol 0, one `float`); anything else is ignored.
unsafe extern "C" fn write_port(
    controller: M,
    port: u32,
    size: u32,
    protocol: u32,
    buffer: *const c_void,
) {
    if controller.is_null() || buffer.is_null() || protocol != 0 || size != 4 {
        return;
    }
    // SAFETY: our controller (a live `UiCtl`) and a 4-byte float buffer, per the protocol.
    let (ctl, v) = unsafe {
        (
            &*(controller as *const UiCtl),
            (buffer as *const f32).read_unaligned(),
        )
    };
    if v.is_finite() {
        lock(&ctl.writes).push((port, v));
    }
}

/// `SuilPortIndexFunc`.
unsafe extern "C" fn port_index(controller: M, symbol: *const c_char) -> u32 {
    if controller.is_null() || symbol.is_null() {
        return INVALID_PORT;
    }
    // SAFETY: our controller and a NUL-terminated symbol.
    let (ctl, symbol) = unsafe {
        (
            &*(controller as *const UiCtl),
            std::ffi::CStr::from_ptr(symbol),
        )
    };
    ctl.symbols
        .iter()
        .find(|(s, _)| s.as_c_str() == symbol)
        .map_or(INVALID_PORT, |(_, i)| *i)
}

/// `ui:resize`'s `ui_resize` (the UI asks for another window size).
unsafe extern "C" fn ui_resize(handle: M, width: c_int, height: c_int) -> c_int {
    if handle.is_null() || width <= 0 || height <= 0 {
        return 1;
    }
    // SAFETY: our `UiCtl`.
    let ctl = unsafe { &*(handle as *const UiCtl) };
    ctl.resize.store(
        (u64::from(width as u32) << 32) | u64::from(height as u32),
        Ordering::Release,
    );
    0
}

/// An open plugin UI (main thread only; freed on drop).
pub(crate) struct UiSession {
    suil: &'static Suil,
    host: M,
    instance: M,
    handle: M,
    idle: Option<unsafe extern "C" fn(M) -> c_int>,
    pub(crate) ctl: Box<UiCtl>,
    /// `(port, last value bits sent)`; `None`: not sent yet.
    sent: Vec<(u32, Option<u32>)>,
    // Kept alive (the UI holds pointers into them) — dropped after `Drop::drop` freed it.
    _resize: Box<LV2UI_Resize>,
    _features: Box<[LV2_Feature; 5]>,
    _feature_ptrs: Box<[*const LV2_Feature; 6]>,
    _urids: Box<Host>,
}

// SAFETY: created, driven and dropped on the sandbox's main thread only (under the instance's
// mutex); the raw pointers are suil's objects for this UI.
unsafe impl Send for UiSession {}

impl UiSession {
    /// Instantiates `choice` for `plugin_uri` inside X11 window `parent`. `symbols`: every port's
    /// `(symbol, index)`; `controls`: the control ports whose values the UI gets.
    pub(crate) fn open(
        suil: &'static Suil,
        choice: &UiChoice,
        plugin_uri: &str,
        parent: u64,
        symbols: Vec<(String, u32)>,
        controls: Vec<u32>,
    ) -> Result<Self, String> {
        let ctl = Box::new(UiCtl::new(symbols));
        let ctl_ptr = (&raw const *ctl).cast_mut().cast::<c_void>();
        let urids = Host::new(48_000.0, 4096, 8192);
        let resize = Box::new(LV2UI_Resize {
            handle: ctl_ptr,
            ui_resize: Some(ui_resize),
        });
        let features = Box::new([
            LV2_Feature {
                URI: uri::UI_PARENT.as_ptr(),
                data: parent as usize as *mut c_void,
            },
            LV2_Feature {
                URI: uri::UI_RESIZE.as_ptr(),
                data: (&raw const *resize).cast_mut().cast(),
            },
            LV2_Feature {
                URI: uri::URID_MAP.as_ptr(),
                data: urids.map().cast_mut().cast(),
            },
            LV2_Feature {
                URI: uri::URID_UNMAP.as_ptr(),
                data: urids.unmap().cast_mut().cast(),
            },
            LV2_Feature {
                URI: uri::UI_IDLE_INTERFACE.as_ptr(),
                data: std::ptr::null_mut(),
            },
        ]);
        let mut feature_ptrs = Box::new([std::ptr::null::<LV2_Feature>(); 6]);
        for (slot, f) in feature_ptrs.iter_mut().zip(features.iter()) {
            *slot = f;
        }
        let c = |s: &str| CString::new(s).map_err(|_| "bad LV2 UI description".to_owned());
        let (plugin, ui, ty, bundle, binary) = (
            c(plugin_uri)?,
            c(&choice.uri)?,
            c(&choice.type_uri)?,
            c(&choice.bundle_path)?,
            c(&choice.binary_path)?,
        );
        // SAFETY: suil calls on the main thread; every string and feature outlives the call
        // (the features and controller outlive the UI: they're fields of the session).
        unsafe {
            let host = (suil.host_new)(Some(write_port), Some(port_index), None, None);
            if host.is_null() {
                return Err("suil couldn't create a UI host".into());
            }
            let instance = (suil.instance_new)(
                host,
                ctl_ptr,
                uri::UI_X11_UI.as_ptr(),
                plugin.as_ptr(),
                ui.as_ptr(),
                ty.as_ptr(),
                bundle.as_ptr(),
                binary.as_ptr(),
                feature_ptrs.as_ptr(),
            );
            if instance.is_null() {
                (suil.host_free)(host);
                return Err("the plugin's window couldn't be created".into());
            }
            let handle = (suil.instance_get_handle)(instance);
            let idle = ((suil.instance_extension_data)(instance, uri::UI_IDLE_INTERFACE.as_ptr())
                as *const LV2UI_Idle_Interface)
                .as_ref()
                .and_then(|i| i.idle);
            Ok(Self {
                suil,
                host,
                instance,
                handle,
                idle,
                ctl,
                sent: controls.into_iter().map(|p| (p, None)).collect(),
                _resize: resize,
                _features: features,
                _feature_ptrs: feature_ptrs,
                _urids: urids,
            })
        }
    }

    /// Sends the UI every control value that changed since it last got one (`value(port)`: the
    /// current value).
    pub(crate) fn update(&mut self, value: impl Fn(u32) -> f32) {
        for (port, sent) in &mut self.sent {
            let v = value(*port);
            if *sent != Some(v.to_bits()) {
                *sent = Some(v.to_bits());
                // SAFETY: main thread; a live UI; a 4-byte float for protocol 0.
                unsafe {
                    (self.suil.instance_port_event)(
                        self.instance,
                        *port,
                        4,
                        0,
                        (&raw const v).cast(),
                    );
                }
            }
        }
    }

    /// One idle round; `false` when the UI asks to close.
    pub(crate) fn idle(&self) -> bool {
        match self.idle {
            // SAFETY: main thread; the UI's own handle.
            Some(f) => unsafe { f(self.handle) == 0 },
            None => true,
        }
    }
}

impl Drop for UiSession {
    fn drop(&mut self) {
        // SAFETY: the UI and host this session created, freed once, UI first.
        unsafe {
            (self.suil.instance_free)(self.instance);
            (self.suil.host_free)(self.host);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ui_callbacks_collect_writes_indices_and_resizes() {
        let ctl = UiCtl::new(vec![("gain".into(), 3), ("mix".into(), 5)]);
        let p = (&raw const ctl).cast_mut().cast::<c_void>();
        let v = -6.5f32;
        // SAFETY: our controller and valid buffers.
        unsafe {
            write_port(p, 3, 4, 0, (&raw const v).cast());
            // Another protocol, a wrong size and a non-finite value are ignored.
            write_port(p, 3, 4, 7, (&raw const v).cast());
            write_port(p, 3, 8, 0, (&raw const v).cast());
            let nan = f32::NAN;
            write_port(p, 5, 4, 0, (&raw const nan).cast());
            assert_eq!(port_index(p, c"mix".as_ptr()), 5);
            assert_eq!(port_index(p, c"nope".as_ptr()), INVALID_PORT);
            assert_eq!(ui_resize(p, 400, 300), 0);
            assert_eq!(ui_resize(p, 0, 300), 1);
        }
        assert_eq!(ctl.take_writes(), vec![(3, -6.5)]);
        assert!(ctl.take_writes().is_empty());
        assert_eq!(ctl.take_resize(), Some((400, 300)));
        assert_eq!(ctl.take_resize(), None);
    }

    #[test]
    fn suil_loads_when_installed_or_says_why() {
        match api() {
            Ok(s) => {
                // SAFETY: static C strings.
                let q =
                    unsafe { (s.ui_supported)(uri::UI_X11_UI.as_ptr(), uri::UI_X11_UI.as_ptr()) };
                assert_eq!(q, 1, "an X11 UI embeds natively in an X11 host");
            }
            Err(e) => assert!(e.starts_with(SUIL_MISSING), "{e}"),
        }
    }
}

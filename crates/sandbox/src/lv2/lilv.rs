//! **lilv, loaded at run time** (ADR-007 amendment, ADR-008 Amendment 8): a hand-written table of
//! the lilv 0.24+ functions the LV2 backend calls, resolved with `libloading` from the system's
//! `liblilv-0` the first time an LV2 plugin is scanned or loaded — in this sandbox process only.
//! PowerVoice never links lilv (no build-time dependency, nothing to install to build it); when
//! the library is missing, every LV2 scan and load fails with [`LILV_MISSING`] and nothing else is
//! affected.
//!
//! lilv's own instance functions (`lilv_instance_run`, …) are `static inline` wrappers around the
//! `LV2_Descriptor`; the backend calls the descriptor directly through
//! [`LilvInstanceImpl`], the public struct lilv documents for exactly that.
//!
//! Every lilv call happens on the sandbox's main thread (discovery, instantiation, state); the
//! audio thread only calls the plugin's descriptor.

use std::ffi::{CStr, CString, OsString, c_char, c_int, c_uint, c_void};
use std::path::Path;
use std::sync::OnceLock;

use vox_lv2_abi::{LV2_Descriptor, LV2_Feature, LV2_Handle, LV2_URID_Map};

/// The message of every LV2 scan or load when lilv can't be loaded.
pub const LILV_MISSING: &str =
    "LV2 support needs the lilv library (liblilv-0), which isn't installed";

/// `struct LilvInstanceImpl` (lilv.h): what `lilv_plugin_instantiate` returns.
#[repr(C)]
pub(crate) struct LilvInstanceImpl {
    pub(crate) lv2_descriptor: *const LV2_Descriptor,
    pub(crate) lv2_handle: LV2_Handle,
    pub(crate) pimpl: *mut c_void,
}

/// `LilvSetPortValueFunc`.
pub(crate) type SetPortValueFn = unsafe extern "C" fn(
    port_symbol: *const c_char,
    user_data: *mut c_void,
    value: *const c_void,
    size: u32,
    type_: u32,
);

/// `LilvUISupportedFunc` (T-901: suil's `suil_ui_supported`).
pub(crate) type UiSupportedFn =
    unsafe extern "C" fn(container_type_uri: *const c_char, ui_type_uri: *const c_char) -> c_uint;

/// Opaque lilv objects (`LilvWorld`, `LilvNode`, `LilvNodes`, `LilvPlugin`, …).
type P = *const c_void;
type M = *mut c_void;

macro_rules! lilv_api {
    ($($field:ident : fn($($arg:ty),*) $(-> $ret:ty)?;)*) => {
        /// The resolved lilv functions (field `x` is `lilv_x`).
        pub(crate) struct Api {
            $(pub(crate) $field: unsafe extern "C" fn($($arg),*) $(-> $ret)?,)*
            _lib: libloading::Library,
        }

        impl Api {
            fn resolve(lib: libloading::Library) -> Result<Self, String> {
                $(
                    let name = concat!("lilv_", stringify!($field), "\0");
                    // SAFETY: the symbol's type is lilv's documented C signature (lilv.h,
                    // 0.24 and later); the pointer stays valid while `_lib` is loaded.
                    let $field = unsafe {
                        lib.get::<unsafe extern "C" fn($($arg),*) $(-> $ret)?>(name.as_bytes())
                    }
                    .map(|s| *s)
                    .map_err(|e| format!("{LILV_MISSING}: {} is missing ({e})", &name[..name.len() - 1]))?;
                )*
                Ok(Self { $($field,)* _lib: lib })
            }
        }
    };
}

lilv_api! {
    world_new: fn() -> M;
    world_free: fn(M);
    world_load_bundle: fn(M, P);
    world_get_all_plugins: fn(P) -> P;
    world_find_nodes: fn(M, P, P, P) -> M;
    new_uri: fn(M, *const c_char) -> M;
    new_file_uri: fn(M, *const c_char, *const c_char) -> M;
    node_free: fn(M);
    node_is_uri: fn(P) -> bool;
    node_as_uri: fn(P) -> *const c_char;
    node_is_literal: fn(P) -> bool;
    node_as_string: fn(P) -> *const c_char;
    node_is_float: fn(P) -> bool;
    node_as_float: fn(P) -> f32;
    node_is_int: fn(P) -> bool;
    node_as_int: fn(P) -> c_int;
    node_is_bool: fn(P) -> bool;
    node_as_bool: fn(P) -> bool;
    nodes_free: fn(M);
    nodes_begin: fn(P) -> M;
    nodes_get: fn(P, P) -> P;
    nodes_next: fn(P, M) -> M;
    nodes_is_end: fn(P, P) -> bool;
    plugins_begin: fn(P) -> M;
    plugins_get: fn(P, P) -> P;
    plugins_next: fn(P, M) -> M;
    plugins_is_end: fn(P, P) -> bool;
    plugin_verify: fn(P) -> bool;
    plugin_get_uri: fn(P) -> P;
    plugin_get_name: fn(P) -> M;
    plugin_get_value: fn(P, P) -> M;
    plugin_has_feature: fn(P, P) -> bool;
    plugin_get_required_features: fn(P) -> M;
    plugin_get_num_ports: fn(P) -> u32;
    plugin_get_port_by_index: fn(P, u32) -> P;
    plugin_get_author_name: fn(P) -> M;
    plugin_get_author_homepage: fn(P) -> M;
    port_get: fn(P, P, P) -> M;
    port_has_property: fn(P, P, P) -> bool;
    port_is_a: fn(P, P, P) -> bool;
    port_get_symbol: fn(P, P) -> P;
    port_get_name: fn(P, P) -> M;
    port_get_range: fn(P, P, *mut M, *mut M, *mut M);
    port_get_scale_points: fn(P, P) -> M;
    scale_points_free: fn(M);
    scale_points_begin: fn(P) -> M;
    scale_points_get: fn(P, P) -> P;
    scale_points_next: fn(P, M) -> M;
    scale_points_is_end: fn(P, P) -> bool;
    scale_point_get_label: fn(P) -> P;
    scale_point_get_value: fn(P) -> P;
    plugin_instantiate: fn(P, f64, *const *const LV2_Feature) -> *mut LilvInstanceImpl;
    instance_free: fn(*mut LilvInstanceImpl);
    state_new_from_world: fn(M, *const LV2_URID_Map, P) -> M;
    state_restore: fn(P, *mut LilvInstanceImpl, Option<SetPortValueFn>, M, u32, *const *const LV2_Feature);
    state_free: fn(M);
    // T-901: plugin UIs.
    plugin_get_uis: fn(P) -> M;
    uis_free: fn(M);
    uis_begin: fn(P) -> M;
    uis_get: fn(P, P) -> P;
    uis_next: fn(P, M) -> M;
    uis_is_end: fn(P, P) -> bool;
    ui_get_uri: fn(P) -> P;
    ui_is_supported: fn(P, Option<UiSupportedFn>, P, *mut P) -> c_uint;
    ui_get_bundle_uri: fn(P) -> P;
    ui_get_binary_uri: fn(P) -> P;
    file_uri_parse: fn(*const c_char, *mut *mut c_char) -> *mut c_char;
    free: fn(*mut c_void);
}

/// Where to look for lilv: `POWERVOICE_LILV` (a path) first, then the platform's names.
fn candidates() -> Vec<OsString> {
    let mut v: Vec<OsString> = std::env::var_os("POWERVOICE_LILV").into_iter().collect();
    let names: &[&str] = if cfg!(target_os = "macos") {
        &[
            "liblilv-0.0.dylib",
            "liblilv-0.dylib",
            "/opt/homebrew/lib/liblilv-0.0.dylib",
            "/usr/local/lib/liblilv-0.0.dylib",
        ]
    } else {
        &["liblilv-0.so.0", "liblilv-0.so"]
    };
    v.extend(names.iter().map(OsString::from));
    v
}

/// Loads lilv from the first of `candidates` that opens and has every function.
pub(crate) fn load_from(candidates: &[OsString]) -> Result<Api, String> {
    let mut last = String::from("no candidate");
    for c in candidates {
        // SAFETY: lilv is a well-behaved system library; its initialisers only run in this
        // disposable sandbox process (ADR-008 §1).
        match unsafe { libloading::Library::new(c) } {
            Ok(lib) => return Api::resolve(lib),
            Err(e) => last = e.to_string(),
        }
    }
    Err(format!("{LILV_MISSING} ({last})"))
}

static API: OnceLock<Result<Api, String>> = OnceLock::new();

/// The process's lilv, loaded on first use.
pub(crate) fn api() -> Result<&'static Api, String> {
    API.get_or_init(|| load_from(&candidates()))
        .as_ref()
        .map_err(Clone::clone)
}

/// A `const char*` lilv returned (borrowed), as an owned string.
///
/// # Safety
/// `s` is null or NUL-terminated.
unsafe fn string(s: *const c_char) -> Option<String> {
    // SAFETY: caller's contract.
    (!s.is_null()).then(|| unsafe { CStr::from_ptr(s) }.to_string_lossy().into_owned())
}

/// An owned `LilvNode` (freed on drop).
pub(crate) struct Node {
    ptr: M,
    api: &'static Api,
}

impl Node {
    /// Takes ownership of `ptr` (`None` if null).
    pub(crate) fn own(api: &'static Api, ptr: M) -> Option<Self> {
        (!ptr.is_null()).then_some(Self { ptr, api })
    }

    pub(crate) fn ptr(&self) -> P {
        self.ptr
    }
}

impl Drop for Node {
    fn drop(&mut self) {
        // SAFETY: a node lilv allocated for us, freed once.
        unsafe { (self.api.node_free)(self.ptr) };
    }
}

/// An owned `LilvNodes` (freed on drop).
pub(crate) struct Nodes {
    ptr: M,
    api: &'static Api,
}

impl Nodes {
    pub(crate) fn own(api: &'static Api, ptr: M) -> Option<Self> {
        (!ptr.is_null()).then_some(Self { ptr, api })
    }

    /// The members (borrowed: valid while `self` lives).
    pub(crate) fn items(&self) -> Vec<P> {
        let api = self.api;
        let mut out = Vec::new();
        // SAFETY: lilv's collection iteration over a live collection.
        unsafe {
            let mut it = (api.nodes_begin)(self.ptr);
            while !(api.nodes_is_end)(self.ptr, it) {
                out.push((api.nodes_get)(self.ptr, it));
                it = (api.nodes_next)(self.ptr, it);
            }
        }
        out
    }
}

impl Drop for Nodes {
    fn drop(&mut self) {
        // SAFETY: a collection lilv allocated for us, freed once.
        unsafe { (self.api.nodes_free)(self.ptr) };
    }
}

/// Node helpers over borrowed nodes.
impl Api {
    /// A URI node's URI.
    pub(crate) fn uri_of(&self, node: P) -> Option<String> {
        // SAFETY: a live node; lilv returns a borrowed NUL-terminated string.
        unsafe {
            (!node.is_null() && (self.node_is_uri)(node))
                .then(|| string((self.node_as_uri)(node)))
                .flatten()
        }
    }

    /// A literal's (or URI's) text.
    pub(crate) fn text_of(&self, node: P) -> Option<String> {
        if node.is_null() {
            return None;
        }
        // SAFETY: a live node; lilv returns a borrowed NUL-terminated string.
        unsafe { string((self.node_as_string)(node)) }
    }

    /// A numeric (or boolean) literal's value.
    pub(crate) fn number_of(&self, node: P) -> Option<f64> {
        if node.is_null() {
            return None;
        }
        // SAFETY: a live node.
        unsafe {
            if (self.node_is_int)(node) {
                Some(f64::from((self.node_as_int)(node)))
            } else if (self.node_is_float)(node) {
                Some(f64::from((self.node_as_float)(node)))
            } else if (self.node_is_bool)(node) {
                Some(if (self.node_as_bool)(node) { 1.0 } else { 0.0 })
            } else if (self.node_is_literal)(node) {
                string((self.node_as_string)(node)).and_then(|s| s.trim().parse().ok())
            } else {
                None
            }
        }
        .filter(|v: &f64| v.is_finite())
    }
}

/// A `LilvWorld` holding one bundle (freed on drop, after everything that borrows from it).
pub(crate) struct World {
    pub(crate) ptr: M,
    pub(crate) api: &'static Api,
}

// SAFETY: a world is only ever used from the sandbox's main thread (discovery, instantiation,
// state); it moves into the plugin instance, which the main thread owns.
unsafe impl Send for World {}
// SAFETY: see `Send` — the audio thread never touches it.
unsafe impl Sync for World {}

impl World {
    /// A new, empty world.
    pub(crate) fn new(api: &'static Api) -> Result<Self, String> {
        // SAFETY: plain constructor.
        let ptr = unsafe { (api.world_new)() };
        if ptr.is_null() {
            return Err("lilv couldn't create a world".into());
        }
        Ok(Self { ptr, api })
    }

    /// A new URI node.
    pub(crate) fn uri(&self, uri: &CStr) -> Node {
        // SAFETY: a live world and a NUL-terminated URI.
        Node::own(self.api, unsafe {
            (self.api.new_uri)(self.ptr, uri.as_ptr())
        })
        .expect("lilv_new_uri never fails for a valid URI")
    }

    /// Loads the bundle directory `path` (its `manifest.ttl` and the data it refers to).
    pub(crate) fn load_bundle(&self, path: &Path) -> Result<(), String> {
        if !path.join("manifest.ttl").is_file() {
            return Err(format!(
                "{} is not an LV2 bundle (no manifest.ttl)",
                path.display()
            ));
        }
        let mut text = path
            .to_str()
            .ok_or_else(|| format!("{} is not a UTF-8 path", path.display()))?
            .to_owned();
        if !text.ends_with('/') {
            text.push('/');
        }
        let c = CString::new(text).map_err(|_| format!("bad bundle path {}", path.display()))?;
        // SAFETY: a live world; a NUL-terminated absolute path (lilv makes the file URI).
        let node = Node::own(self.api, unsafe {
            (self.api.new_file_uri)(self.ptr, std::ptr::null(), c.as_ptr())
        })
        .ok_or_else(|| format!("bad bundle path {}", path.display()))?;
        // SAFETY: a live world and a URI node (lilv copies what it keeps).
        unsafe { (self.api.world_load_bundle)(self.ptr, node.ptr()) };
        Ok(())
    }

    /// Every plugin the loaded bundles describe (borrowed from the world).
    pub(crate) fn plugins(&self) -> Vec<P> {
        let api = self.api;
        let mut out = Vec::new();
        // SAFETY: lilv's collection iteration over the world's plugin set.
        unsafe {
            let all = (api.world_get_all_plugins)(self.ptr);
            let mut it = (api.plugins_begin)(all);
            while !(api.plugins_is_end)(all, it) {
                out.push((api.plugins_get)(all, it));
                it = (api.plugins_next)(all, it);
            }
        }
        out
    }

    /// The plugin with URI `uri`.
    pub(crate) fn plugin(&self, uri: &str) -> Option<P> {
        self.plugins()
            .into_iter()
            .find(|&p| self.plugin_uri(p).as_deref() == Some(uri))
    }

    /// A plugin's URI.
    pub(crate) fn plugin_uri(&self, plugin: P) -> Option<String> {
        // SAFETY: a plugin of this world; the URI node is borrowed.
        self.api
            .uri_of(unsafe { (self.api.plugin_get_uri)(plugin) })
    }

    /// `(subject, predicate, *)` objects.
    pub(crate) fn find(&self, subject: P, predicate: &CStr) -> Option<Nodes> {
        let pred = self.uri(predicate);
        // SAFETY: a live world and live nodes.
        Nodes::own(self.api, unsafe {
            (self.api.world_find_nodes)(self.ptr, subject, pred.ptr(), std::ptr::null())
        })
    }

    /// A plugin's values for `predicate`.
    pub(crate) fn plugin_values(&self, plugin: P, predicate: &CStr) -> Option<Nodes> {
        let pred = self.uri(predicate);
        // SAFETY: a plugin of this world and a live node.
        Nodes::own(self.api, unsafe {
            (self.api.plugin_get_value)(plugin, pred.ptr())
        })
    }

    /// A plugin's first value for `predicate`.
    pub(crate) fn plugin_first_text(&self, plugin: P, predicate: &CStr) -> Option<String> {
        let nodes = self.plugin_values(plugin, predicate)?;
        nodes.items().first().and_then(|&n| self.api.text_of(n))
    }
}

/// A plugin UI PowerVoice can show (T-901): what suil needs to instantiate it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct UiChoice {
    /// The UI's URI.
    pub(crate) uri: String,
    /// Its widget type (e.g. `ui:X11UI`).
    pub(crate) type_uri: String,
    /// Its bundle directory.
    pub(crate) bundle_path: String,
    /// Its shared library.
    pub(crate) binary_path: String,
}

impl World {
    /// The plugin's first UI that `supported` (suil) can show in a `container` (T-901: an
    /// `ui:X11UI` window).
    pub(crate) fn plugin_ui(
        &self,
        plugin: P,
        container: &CStr,
        supported: UiSupportedFn,
    ) -> Option<UiChoice> {
        let api = self.api;
        let container = self.uri(container);
        // SAFETY: a plugin of this world; the UI collection is ours (freed below), its members
        // and their nodes are borrowed from it.
        unsafe {
            let uis = (api.plugin_get_uis)(plugin);
            if uis.is_null() {
                return None;
            }
            let mut found = None;
            let mut it = (api.uis_begin)(uis);
            while found.is_none() && !(api.uis_is_end)(uis, it) {
                let ui = (api.uis_get)(uis, it);
                let mut ty: P = std::ptr::null();
                if (api.ui_is_supported)(ui, Some(supported), container.ptr(), &mut ty) > 0 {
                    found = Some(UiChoice {
                        uri: api.uri_of((api.ui_get_uri)(ui)).unwrap_or_default(),
                        type_uri: api.uri_of(ty).unwrap_or_default(),
                        bundle_path: self
                            .path_of((api.ui_get_bundle_uri)(ui))
                            .unwrap_or_default(),
                        binary_path: self
                            .path_of((api.ui_get_binary_uri)(ui))
                            .unwrap_or_default(),
                    })
                    .filter(|c| {
                        !c.uri.is_empty() && !c.type_uri.is_empty() && !c.binary_path.is_empty()
                    });
                }
                it = (api.uis_next)(uis, it);
            }
            (api.uis_free)(uis);
            found
        }
    }

    /// The local path of a `file:` URI node.
    fn path_of(&self, node: P) -> Option<String> {
        let uri = CString::new(self.api.uri_of(node)?).ok()?;
        // SAFETY: a NUL-terminated URI; lilv returns an allocated string (or null), freed with
        // `lilv_free`.
        unsafe {
            let p = (self.api.file_uri_parse)(uri.as_ptr(), std::ptr::null_mut());
            if p.is_null() {
                return None;
            }
            let s = CStr::from_ptr(p).to_string_lossy().into_owned();
            (self.api.free)(p.cast());
            Some(s)
        }
    }
}

impl Drop for World {
    fn drop(&mut self) {
        // SAFETY: a world we created, freed once, after its borrowed plugins/nodes are gone.
        unsafe { (self.api.world_free)(self.ptr) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_library_is_a_clear_error() {
        let err = load_from(&[OsString::from("/nonexistent/liblilv-0.so.0")])
            .err()
            .expect("no such library");
        assert!(err.starts_with(LILV_MISSING), "{err}");
    }
}

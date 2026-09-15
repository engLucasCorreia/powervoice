//! Hand-written Rust bindings for the subset of the **LV2** C API PowerVoice uses (T-807,
//! ADR-008 Amendment 8): the plugin descriptor and features (`lv2core`), `urid`, the old
//! `uri-map`, `options`, `worker`, `state`, and the atom headers the host fills in for atom
//! ports. Both sides use them: the sandbox's LV2 host (`powervoice-sandbox`) and the in-repo test
//! plugin (`vox-test-lv2`).
//!
//! The definitions mirror the LV2 headers (https://lv2plug.in, ISC — see `LICENSE-LV2` in this
//! crate) field by field; names keep the C spelling so they can be checked against the headers
//! side by side. Function pointers are `Option<…>` (same ABI as a nullable C function pointer).
//! C enums are `u32` (4 bytes, as the headers' enums are on every supported ABI). Layout tests
//! pin the sizes on 64-bit targets.
//!
//! lilv (plugin discovery and instantiation) is **not** bound here: the sandbox loads it at run
//! time with `libloading` (ADR-007 amendment, T-807).

#![allow(non_camel_case_types, non_snake_case)]

use std::ffi::{CStr, c_char, c_void};

/// `LV2_Handle`: an instance's opaque handle.
pub type LV2_Handle = *mut c_void;

/// `LV2_Feature`: a host feature passed to `instantiate` (or to state save/restore).
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct LV2_Feature {
    /// The feature's URI.
    pub URI: *const c_char,
    /// Feature data (feature-specific; may be null).
    pub data: *mut c_void,
}

/// `LV2_Descriptor`: one plugin of a library.
#[repr(C)]
pub struct LV2_Descriptor {
    /// The plugin's URI.
    pub URI: *const c_char,
    /// \[instantiation\] Creates an instance.
    pub instantiate: Option<
        unsafe extern "C" fn(
            descriptor: *const LV2_Descriptor,
            sample_rate: f64,
            bundle_path: *const c_char,
            features: *const *const LV2_Feature,
        ) -> LV2_Handle,
    >,
    /// \[audio\] Connects a port to a buffer (real-time safe).
    pub connect_port:
        Option<unsafe extern "C" fn(instance: LV2_Handle, port: u32, data_location: *mut c_void)>,
    /// \[instantiation\] Resets and prepares for `run`.
    pub activate: Option<unsafe extern "C" fn(instance: LV2_Handle)>,
    /// \[audio\] Processes `sample_count` frames.
    pub run: Option<unsafe extern "C" fn(instance: LV2_Handle, sample_count: u32)>,
    /// \[instantiation\] Counterpart of `activate`.
    pub deactivate: Option<unsafe extern "C" fn(instance: LV2_Handle)>,
    /// \[instantiation\] Destroys the instance.
    pub cleanup: Option<unsafe extern "C" fn(instance: LV2_Handle)>,
    /// \[discovery\] Extension data (`state:interface`, `worker:interface`, …) or null.
    pub extension_data: Option<unsafe extern "C" fn(uri: *const c_char) -> *const c_void>,
}

/// `lv2_descriptor`'s type (the library's entry symbol).
pub type LV2_Descriptor_Function = unsafe extern "C" fn(index: u32) -> *const LV2_Descriptor;

/// The entry symbol a plugin library exports.
pub const LV2_DESCRIPTOR_SYMBOL: &[u8] = b"lv2_descriptor\0";

// --- urid, uri-map ----------------------------------------------------------------------------

/// `LV2_URID`.
pub type LV2_URID = u32;

/// `LV2_URID_Map` (feature data of `urid:map`).
#[repr(C)]
pub struct LV2_URID_Map {
    /// Host data.
    pub handle: *mut c_void,
    /// Maps a URI to an id (thread-safe; 0 = failure).
    pub map: Option<unsafe extern "C" fn(handle: *mut c_void, uri: *const c_char) -> LV2_URID>,
}

/// `LV2_URID_Unmap` (feature data of `urid:unmap`).
#[repr(C)]
pub struct LV2_URID_Unmap {
    /// Host data.
    pub handle: *mut c_void,
    /// The URI of an id (null if unknown).
    pub unmap: Option<unsafe extern "C" fn(handle: *mut c_void, urid: LV2_URID) -> *const c_char>,
}

/// `LV2_URI_Map_Feature` (the deprecated `uri-map` extension, still used by older plugins).
#[repr(C)]
pub struct LV2_URI_Map_Feature {
    /// Host data.
    pub callback_data: *mut c_void,
    /// Maps `uri` (in the context of extension `map`, ignored) to an id.
    pub uri_to_id: Option<
        unsafe extern "C" fn(
            callback_data: *mut c_void,
            map: *const c_char,
            uri: *const c_char,
        ) -> u32,
    >,
}

// --- options -----------------------------------------------------------------------------------

/// `LV2_OPTIONS_INSTANCE`: an option about the instance.
pub const LV2_OPTIONS_INSTANCE: u32 = 0;

/// `LV2_Options_Option`: one entry of the `options:options` array (terminated by an all-zero
/// entry).
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct LV2_Options_Option {
    /// `LV2_Options_Context`.
    pub context: u32,
    /// Subject (0 for the instance).
    pub subject: u32,
    /// Key URID.
    pub key: LV2_URID,
    /// Value size in bytes.
    pub size: u32,
    /// Value type URID (`atom:Int`, `atom:Float`, …).
    pub type_: LV2_URID,
    /// Value.
    pub value: *const c_void,
}

// --- worker -------------------------------------------------------------------------------------

/// `LV2_WORKER_SUCCESS`.
pub const LV2_WORKER_SUCCESS: u32 = 0;
/// `LV2_WORKER_ERR_UNKNOWN`.
pub const LV2_WORKER_ERR_UNKNOWN: u32 = 1;
/// `LV2_WORKER_ERR_NO_SPACE`.
pub const LV2_WORKER_ERR_NO_SPACE: u32 = 2;

/// `LV2_Worker_Respond_Function`.
pub type LV2_Worker_Respond_Function =
    unsafe extern "C" fn(handle: *mut c_void, size: u32, data: *const c_void) -> u32;

/// `LV2_Worker_Interface` (extension data of `worker:interface`).
#[repr(C)]
pub struct LV2_Worker_Interface {
    /// \[worker\] Does the work; answers through `respond`.
    pub work: Option<
        unsafe extern "C" fn(
            instance: LV2_Handle,
            respond: LV2_Worker_Respond_Function,
            handle: *mut c_void,
            size: u32,
            data: *const c_void,
        ) -> u32,
    >,
    /// \[audio\] Delivers a response.
    pub work_response:
        Option<unsafe extern "C" fn(instance: LV2_Handle, size: u32, body: *const c_void) -> u32>,
    /// \[audio\] Called after `run` (and the responses of that cycle).
    pub end_run: Option<unsafe extern "C" fn(instance: LV2_Handle) -> u32>,
}

/// `LV2_Worker_Schedule` (feature data of `worker:schedule`).
#[repr(C)]
pub struct LV2_Worker_Schedule {
    /// Host data.
    pub handle: *mut c_void,
    /// \[audio\] Schedules work (copied by the host).
    pub schedule_work:
        Option<unsafe extern "C" fn(handle: *mut c_void, size: u32, data: *const c_void) -> u32>,
}

// --- state --------------------------------------------------------------------------------------

/// `LV2_STATE_IS_POD`.
pub const LV2_STATE_IS_POD: u32 = 1;
/// `LV2_STATE_IS_PORTABLE`.
pub const LV2_STATE_IS_PORTABLE: u32 = 1 << 1;
/// `LV2_STATE_IS_NATIVE`.
pub const LV2_STATE_IS_NATIVE: u32 = 1 << 2;
/// `LV2_STATE_SUCCESS`.
pub const LV2_STATE_SUCCESS: u32 = 0;
/// `LV2_STATE_ERR_UNKNOWN`.
pub const LV2_STATE_ERR_UNKNOWN: u32 = 1;
/// `LV2_STATE_ERR_NO_PROPERTY`.
pub const LV2_STATE_ERR_NO_PROPERTY: u32 = 5;

/// `LV2_State_Store_Function`.
pub type LV2_State_Store_Function = unsafe extern "C" fn(
    handle: *mut c_void,
    key: u32,
    value: *const c_void,
    size: usize,
    type_: u32,
    flags: u32,
) -> u32;

/// `LV2_State_Retrieve_Function`.
pub type LV2_State_Retrieve_Function = unsafe extern "C" fn(
    handle: *mut c_void,
    key: u32,
    size: *mut usize,
    type_: *mut u32,
    flags: *mut u32,
) -> *const c_void;

/// `LV2_State_Interface` (extension data of `state:interface`).
#[repr(C)]
pub struct LV2_State_Interface {
    /// Saves the instance's state through `store`.
    pub save: Option<
        unsafe extern "C" fn(
            instance: LV2_Handle,
            store: LV2_State_Store_Function,
            handle: *mut c_void,
            flags: u32,
            features: *const *const LV2_Feature,
        ) -> u32,
    >,
    /// Restores a state through `retrieve`.
    pub restore: Option<
        unsafe extern "C" fn(
            instance: LV2_Handle,
            retrieve: LV2_State_Retrieve_Function,
            handle: *mut c_void,
            flags: u32,
            features: *const *const LV2_Feature,
        ) -> u32,
    >,
}

/// `LV2_State_Map_Path` (feature data of `state:mapPath`).
#[repr(C)]
pub struct LV2_State_Map_Path {
    /// Host data.
    pub handle: *mut c_void,
    /// An absolute path as an abstract one (`malloc`ed; freed by the plugin).
    pub abstract_path: Option<
        unsafe extern "C" fn(handle: *mut c_void, absolute_path: *const c_char) -> *mut c_char,
    >,
    /// An abstract path as an absolute one (`malloc`ed; freed by the plugin).
    pub absolute_path: Option<
        unsafe extern "C" fn(handle: *mut c_void, abstract_path: *const c_char) -> *mut c_char,
    >,
}

/// `LV2_State_Free_Path` (feature data of `state:freePath`).
#[repr(C)]
pub struct LV2_State_Free_Path {
    /// Host data.
    pub handle: *mut c_void,
    /// Frees a path the host returned.
    pub free_path: Option<unsafe extern "C" fn(handle: *mut c_void, path: *mut c_char)>,
}

// --- atom ---------------------------------------------------------------------------------------

/// `LV2_Atom`: every atom's header.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LV2_Atom {
    /// Body size in bytes (the header excluded).
    pub size: u32,
    /// Type URID.
    pub type_: u32,
}

/// `LV2_Atom_Sequence_Body`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LV2_Atom_Sequence_Body {
    /// Time unit URID (0: frames).
    pub unit: u32,
    /// Padding.
    pub pad: u32,
}

/// `LV2_Atom_Sequence`: an empty sequence is this header with `atom.size` =
/// `size_of::<LV2_Atom_Sequence_Body>()`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LV2_Atom_Sequence {
    /// Header.
    pub atom: LV2_Atom,
    /// Body header.
    pub body: LV2_Atom_Sequence_Body,
}

// --- URIs ---------------------------------------------------------------------------------------

/// URIs of the LV2 vocabulary PowerVoice's host and test plugin use (from the headers'
/// `#define`s).
pub mod uri {
    use std::ffi::CStr;

    macro_rules! uris {
        ($($name:ident = $value:literal;)*) => {
            $(
                #[doc = $value]
                pub const $name: &CStr = match CStr::from_bytes_with_nul(concat!($value, "\0").as_bytes()) {
                    Ok(c) => c,
                    Err(_) => panic!("URIs have no interior NUL"),
                };
            )*
        };
    }

    uris! {
        // lv2core
        LV2_CORE_PLUGIN = "http://lv2plug.in/ns/lv2core#Plugin";
        AUDIO_PORT = "http://lv2plug.in/ns/lv2core#AudioPort";
        CONTROL_PORT = "http://lv2plug.in/ns/lv2core#ControlPort";
        CV_PORT = "http://lv2plug.in/ns/lv2core#CVPort";
        INPUT_PORT = "http://lv2plug.in/ns/lv2core#InputPort";
        OUTPUT_PORT = "http://lv2plug.in/ns/lv2core#OutputPort";
        INSTRUMENT_PLUGIN = "http://lv2plug.in/ns/lv2core#InstrumentPlugin";
        CONNECTION_OPTIONAL = "http://lv2plug.in/ns/lv2core#connectionOptional";
        DESIGNATION = "http://lv2plug.in/ns/lv2core#designation";
        ENABLED = "http://lv2plug.in/ns/lv2core#enabled";
        ENUMERATION = "http://lv2plug.in/ns/lv2core#enumeration";
        FREE_WHEELING = "http://lv2plug.in/ns/lv2core#freeWheeling";
        HARD_RT_CAPABLE = "http://lv2plug.in/ns/lv2core#hardRTCapable";
        IN_PLACE_BROKEN = "http://lv2plug.in/ns/lv2core#inPlaceBroken";
        INTEGER = "http://lv2plug.in/ns/lv2core#integer";
        IS_LIVE = "http://lv2plug.in/ns/lv2core#isLive";
        IS_SIDE_CHAIN = "http://lv2plug.in/ns/lv2core#isSideChain";
        LATENCY = "http://lv2plug.in/ns/lv2core#latency";
        MINOR_VERSION = "http://lv2plug.in/ns/lv2core#minorVersion";
        MICRO_VERSION = "http://lv2plug.in/ns/lv2core#microVersion";
        NAME = "http://lv2plug.in/ns/lv2core#name";
        REPORTS_LATENCY = "http://lv2plug.in/ns/lv2core#reportsLatency";
        SAMPLE_RATE = "http://lv2plug.in/ns/lv2core#sampleRate";
        TOGGLED = "http://lv2plug.in/ns/lv2core#toggled";
        // rdf / rdfs / doap
        RDF_TYPE = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
        RDFS_COMMENT = "http://www.w3.org/2000/01/rdf-schema#comment";
        RDFS_LABEL = "http://www.w3.org/2000/01/rdf-schema#label";
        DOAP_HOMEPAGE = "http://usefulinc.com/ns/doap#homepage";
        // atom
        ATOM_PORT = "http://lv2plug.in/ns/ext/atom#AtomPort";
        ATOM_CHUNK = "http://lv2plug.in/ns/ext/atom#Chunk";
        ATOM_DOUBLE = "http://lv2plug.in/ns/ext/atom#Double";
        ATOM_FLOAT = "http://lv2plug.in/ns/ext/atom#Float";
        ATOM_INT = "http://lv2plug.in/ns/ext/atom#Int";
        ATOM_SEQUENCE = "http://lv2plug.in/ns/ext/atom#Sequence";
        ATOM_STRING = "http://lv2plug.in/ns/ext/atom#String";
        // event (deprecated)
        EVENT_PORT = "http://lv2plug.in/ns/ext/event#EventPort";
        // urid, uri-map
        URID_MAP = "http://lv2plug.in/ns/ext/urid#map";
        URID_UNMAP = "http://lv2plug.in/ns/ext/urid#unmap";
        URI_MAP = "http://lv2plug.in/ns/ext/uri-map";
        // options, buf-size, parameters
        OPTIONS_OPTIONS = "http://lv2plug.in/ns/ext/options#options";
        OPTIONS_INTERFACE = "http://lv2plug.in/ns/ext/options#interface";
        OPTIONS_REQUIRED_OPTION = "http://lv2plug.in/ns/ext/options#requiredOption";
        BUF_SIZE_BOUNDED_BLOCK_LENGTH = "http://lv2plug.in/ns/ext/buf-size#boundedBlockLength";
        BUF_SIZE_MAX_BLOCK_LENGTH = "http://lv2plug.in/ns/ext/buf-size#maxBlockLength";
        BUF_SIZE_MIN_BLOCK_LENGTH = "http://lv2plug.in/ns/ext/buf-size#minBlockLength";
        BUF_SIZE_NOMINAL_BLOCK_LENGTH = "http://lv2plug.in/ns/ext/buf-size#nominalBlockLength";
        BUF_SIZE_SEQUENCE_SIZE = "http://lv2plug.in/ns/ext/buf-size#sequenceSize";
        PARAM_SAMPLE_RATE = "http://lv2plug.in/ns/ext/parameters#sampleRate";
        RESIZE_PORT_MINIMUM_SIZE = "http://lv2plug.in/ns/ext/resize-port#minimumSize";
        // worker
        WORKER_INTERFACE = "http://lv2plug.in/ns/ext/worker#interface";
        WORKER_SCHEDULE = "http://lv2plug.in/ns/ext/worker#schedule";
        // state
        STATE_INTERFACE = "http://lv2plug.in/ns/ext/state#interface";
        STATE_LOAD_DEFAULT_STATE = "http://lv2plug.in/ns/ext/state#loadDefaultState";
        STATE_MAP_PATH = "http://lv2plug.in/ns/ext/state#mapPath";
        STATE_FREE_PATH = "http://lv2plug.in/ns/ext/state#freePath";
        STATE_THREAD_SAFE_RESTORE = "http://lv2plug.in/ns/ext/state#threadSafeRestore";
        // port-props, port-groups, units
        PORT_PROPS_LOGARITHMIC = "http://lv2plug.in/ns/ext/port-props#logarithmic";
        PORT_PROPS_NOT_AUTOMATIC = "http://lv2plug.in/ns/ext/port-props#notAutomatic";
        PORT_PROPS_NOT_ON_GUI = "http://lv2plug.in/ns/ext/port-props#notOnGUI";
        PORT_GROUPS_GROUP = "http://lv2plug.in/ns/ext/port-groups#group";
        UNITS_UNIT = "http://lv2plug.in/ns/extensions/units#unit";
    }

    /// The `units:` namespace (`units:db`, `units:hz`, …).
    pub const UNITS_PREFIX: &str = "http://lv2plug.in/ns/extensions/units#";
    /// The `lv2core` namespace (plugin classes: `lv2:EQPlugin`, …).
    pub const LV2_CORE_PREFIX: &str = "http://lv2plug.in/ns/lv2core#";
}

/// `uri` as a `&str` (every constant in [`uri`] is ASCII).
pub fn uri_str(uri: &CStr) -> &str {
    uri.to_str().unwrap_or_default()
}

#[cfg(all(test, target_pointer_width = "64"))]
mod tests {
    use super::*;
    use std::mem::{offset_of, size_of};

    #[test]
    fn layouts_match_the_headers_on_64_bit() {
        assert_eq!(size_of::<LV2_Feature>(), 16);
        assert_eq!(size_of::<LV2_Descriptor>(), 64);
        assert_eq!(offset_of!(LV2_Descriptor, run), 32);
        assert_eq!(offset_of!(LV2_Descriptor, extension_data), 56);
        assert_eq!(size_of::<LV2_URID_Map>(), 16);
        assert_eq!(size_of::<LV2_URID_Unmap>(), 16);
        assert_eq!(size_of::<LV2_URI_Map_Feature>(), 16);
        assert_eq!(size_of::<LV2_Options_Option>(), 32);
        assert_eq!(offset_of!(LV2_Options_Option, value), 24);
        assert_eq!(size_of::<LV2_Worker_Interface>(), 24);
        assert_eq!(size_of::<LV2_Worker_Schedule>(), 16);
        assert_eq!(size_of::<LV2_State_Interface>(), 16);
        assert_eq!(size_of::<LV2_State_Map_Path>(), 24);
        assert_eq!(size_of::<LV2_State_Free_Path>(), 16);
        assert_eq!(size_of::<LV2_Atom>(), 8);
        assert_eq!(size_of::<LV2_Atom_Sequence>(), 16);
    }

    #[test]
    fn uris_are_ascii_and_prefixed() {
        assert_eq!(
            uri_str(uri::AUDIO_PORT),
            "http://lv2plug.in/ns/lv2core#AudioPort"
        );
        assert!(uri_str(uri::UNITS_UNIT).starts_with(uri::UNITS_PREFIX));
        assert!(uri_str(uri::TOGGLED).starts_with(uri::LV2_CORE_PREFIX));
    }
}

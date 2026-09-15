//! The `org.powervoice.module-info/1` vendor extension (ADR-006 §2), plugin side: one
//! `[main-thread]` call that writes the module's [`vox_module_api::ModuleInfo`] as UTF-8 JSON
//! to a `clap_ostream`. Implemented with clack's own extension tools (the same ones its standard
//! extensions use).

use std::ffi::{CStr, c_void};

use clack_plugin::extensions::prelude::*;
use clack_plugin::plugin::PluginError;
use vox_module_api::ModuleFactory;

use crate::ClapModule;

/// `clap_ostream`, as the CLAP headers define it (clack keeps its `clap-sys` types private to
/// its own API; this struct is ABI-identical).
#[repr(C)]
#[derive(Clone, Copy)]
struct RawOstream {
    ctx: *mut c_void,
    write: Option<unsafe extern "C" fn(*const RawOstream, *const c_void, u64) -> i64>,
}

/// The extension's vtable (same layout as `vox_clap_abi::clap_plugin_module_info`).
#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct RawModuleInfo {
    get_info: Option<unsafe extern "C" fn(*const clap_plugin, *const RawOstream) -> bool>,
}

/// The plugin side of `org.powervoice.module-info/1`.
#[derive(Copy, Clone)]
pub struct PluginModuleInfo(#[allow(dead_code)] RawExtension<PluginExtensionSide, RawModuleInfo>);

// SAFETY: `RawModuleInfo` is `#[repr(C)]` and matches the vtable PowerVoice defines for this id.
unsafe impl Extension for PluginModuleInfo {
    const IDENTIFIERS: &'static [&'static CStr] = &[c"org.powervoice.module-info/1"];
    type ExtensionSide = PluginExtensionSide;

    unsafe fn from_raw(raw: RawExtension<Self::ExtensionSide>) -> Self {
        // SAFETY: the caller guarantees the pointer is this extension's vtable.
        Self(unsafe { raw.cast() })
    }
}

// SAFETY: `IMPLEMENTATION` points at a `'static`, `#[repr(C)]` `RawModuleInfo`.
unsafe impl<F: ModuleFactory + Default + 'static> ExtensionImplementation<ClapModule<F>>
    for PluginModuleInfo
{
    const IMPLEMENTATION: RawExtensionImplementation =
        RawExtensionImplementation::new(&RawModuleInfo {
            get_info: Some(get_info::<F>),
        });
}

/// Writes all of `bytes` to `stream` (a CLAP stream may accept fewer bytes per call).
///
/// # Safety
/// `stream` is a valid `clap_ostream` for the duration of the call.
unsafe fn write_all(stream: *const RawOstream, mut bytes: &[u8]) -> Result<(), PluginError> {
    // SAFETY: caller's contract.
    let write = unsafe { (*stream).write }.ok_or(PluginError::Message("stream can't write"))?;
    while !bytes.is_empty() {
        // SAFETY: `bytes` is valid for `len` bytes; the stream is valid (caller's contract).
        let n = unsafe { write(stream, bytes.as_ptr().cast(), bytes.len() as u64) };
        if n <= 0 {
            return Err(PluginError::Message(
                "the host's stream refused the module info",
            ));
        }
        bytes = &bytes[(n as usize).min(bytes.len())..];
    }
    Ok(())
}

unsafe extern "C" fn get_info<F: ModuleFactory + Default + 'static>(
    plugin: *const clap_plugin,
    stream: *const RawOstream,
) -> bool {
    if stream.is_null() {
        return false;
    }
    // SAFETY: `plugin` is the instance the host called us with; clack checks it (null,
    // uninitialised, panics). `stream` is non-null and valid for the call (CLAP contract).
    unsafe {
        PluginWrapper::<ClapModule<F>>::handle(plugin, |p| {
            write_all(stream, &p.shared().info_json)?;
            Ok(())
        })
    }
    .is_some()
}

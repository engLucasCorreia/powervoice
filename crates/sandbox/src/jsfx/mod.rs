//! The **JSFX backend** (T-808, ADR-008 Amendment 11): format `"jsfx"`, plugin reference =
//! [`JsfxPluginRef`](vox_sandbox_ipc::protocol::JsfxPluginRef) (the script's path, JSON).
//!
//! - `fx`: one ysfx effect (the vendored library, compiled into this binary only — ADR-007
//!   §5 and its T-808 amendment); load + compile, sliders, processing, state.
//! - `instance`: one script as a [`PluginInstance`] — sample-accurate sliders by block
//!   splitting, the mono shim over the pins, latency through `pdc_delay`, state.
//! - `params`: sliders → the module API's schema.
//! - `state`: the state bytes.
//! - [`scan`]: `powervoice-sandbox --scan <script> --format jsfx`.
//!
//! `@gfx` is never compiled or run (GUIs are T-901). Built where `vox-ysfx-sys` is (unix,
//! x86-64 and aarch64 — the EEL2 JIT back ends vendored); elsewhere, Windows included, the backend
//! answers [`UNSUPPORTED`].

#[cfg(all(unix, any(target_arch = "x86_64", target_arch = "aarch64")))]
mod fx;
#[cfg(all(unix, any(target_arch = "x86_64", target_arch = "aarch64")))]
mod instance;
#[cfg(all(unix, any(target_arch = "x86_64", target_arch = "aarch64")))]
mod params;
pub mod scan;
#[cfg(all(unix, any(target_arch = "x86_64", target_arch = "aarch64")))]
mod state;

use std::sync::Arc;

use crate::backend::{PluginBackend, PluginInstance};

#[cfg(all(unix, any(target_arch = "x86_64", target_arch = "aarch64")))]
pub use instance::JsfxInstance;

/// Why JSFX effects can't be hosted on this platform.
#[cfg_attr(
    all(unix, any(target_arch = "x86_64", target_arch = "aarch64")),
    allow(dead_code)
)]
pub const UNSUPPORTED: &str = "JSFX effects aren't supported on this platform";

/// Whether JSFX effects can be hosted here, else why not.
pub fn available() -> Result<(), String> {
    if cfg!(all(
        unix,
        any(target_arch = "x86_64", target_arch = "aarch64")
    )) {
        Ok(())
    } else {
        Err(UNSUPPORTED.into())
    }
}

/// The `"jsfx"` backend.
pub struct JsfxBackend;

impl PluginBackend for JsfxBackend {
    fn name(&self) -> &'static str {
        "jsfx"
    }

    #[cfg(all(unix, any(target_arch = "x86_64", target_arch = "aarch64")))]
    fn load(&self, plugin: &str) -> Result<Arc<dyn PluginInstance>, String> {
        let r = vox_sandbox_ipc::protocol::JsfxPluginRef::parse(plugin)?;
        Ok(Arc::new(JsfxInstance::load(std::path::Path::new(&r.path))?))
    }

    #[cfg(not(all(unix, any(target_arch = "x86_64", target_arch = "aarch64"))))]
    fn load(&self, _plugin: &str) -> Result<Arc<dyn PluginInstance>, String> {
        Err(UNSUPPORTED.into())
    }
}

//! The **LV2 backend** (T-807, ADR-008 Amendment 8): format `"lv2"`, plugin reference =
//! [`Lv2PluginRef`] (the `.lv2` bundle directory and the plugin URI, JSON).
//!
//! - [`lilv`]: lilv loaded at run time with `libloading` (never linked — ADR-007 amendment); a
//!   missing `liblilv-0` makes every LV2 scan and load fail with [`lilv::LILV_MISSING`].
//! - [`host`]: the features PowerVoice provides (`urid`, `options`, `worker:schedule`, …).
//! - [`worker`]: the worker's rings and thread.
//! - [`instance`]: one plugin as a [`PluginInstance`] — ports, sample-accurate control by
//!   splitting blocks, the mono shim, latency, state.
//! - [`params`]: ports → the module API's schema.
//! - [`state`]: the state bytes.
//! - [`scan`]: `powervoice-sandbox --scan <bundle> --format lv2`.
//!
//! Unix only (Linux first; macOS compiles and works when lilv is installed). On Windows the
//! backend answers [`UNSUPPORTED`] — the editor doesn't look for LV2 bundles there either.

#[cfg(unix)]
mod host;
#[cfg(unix)]
mod instance;
#[cfg(unix)]
pub mod lilv;
#[cfg(unix)]
mod params;
pub mod scan;
#[cfg(unix)]
mod state;
#[cfg(unix)]
mod suil;
#[cfg(unix)]
mod worker;

use std::sync::Arc;

use crate::backend::{PluginBackend, PluginInstance};

#[cfg(unix)]
pub use instance::Lv2Instance;

/// Why LV2 plugins can't be hosted on this platform.
#[cfg_attr(unix, allow(dead_code))]
pub const UNSUPPORTED: &str = "LV2 plugins aren't supported on this platform";

/// Whether LV2 plugins can be hosted here: lilv loads (unix), else why not.
pub fn available() -> Result<(), String> {
    #[cfg(unix)]
    {
        lilv::api().map(|_| ())
    }
    #[cfg(not(unix))]
    {
        Err(UNSUPPORTED.into())
    }
}

/// The `"lv2"` backend.
pub struct Lv2Backend;

impl PluginBackend for Lv2Backend {
    fn name(&self) -> &'static str {
        "lv2"
    }

    #[cfg(unix)]
    fn load(&self, plugin: &str) -> Result<Arc<dyn PluginInstance>, String> {
        let r = vox_sandbox_ipc::protocol::Lv2PluginRef::parse(plugin)?;
        Ok(Arc::new(Lv2Instance::load(
            std::path::Path::new(&r.path),
            &r.uri,
        )?))
    }

    #[cfg(not(unix))]
    fn load(&self, _plugin: &str) -> Result<Arc<dyn PluginInstance>, String> {
        Err(UNSUPPORTED.into())
    }
}

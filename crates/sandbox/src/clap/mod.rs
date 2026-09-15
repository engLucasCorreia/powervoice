//! The **CLAP backend** (T-803, ADR-008 Amendment 3): format `"clap"`, plugin reference =
//! [`ClapPluginRef`] (the `.clap` path and the plugin id, JSON).
//!
//! - [`library`]: dlopen (`libloading`), `clap_entry.init`/`deinit`, the plugin factory.
//! - [`host`]: the `clap_host` PowerVoice presents (log → the sandbox's stderr, thread check,
//!   `request_restart`/`request_callback`/`request_flush` flags the sandbox acts on).
//! - [`instance`]: one plugin as a [`PluginInstance`] — params, state, latency, tail, audio
//!   ports with the mono shim, sample-accurate parameter events per chunk.
//! - [`scan`]: `powervoice-sandbox --scan <file> --format clap` (descriptors only).
//! - `module_info`: PowerVoice modules packaged as CLAP (T-805, ADR-006 §2) — the
//!   `org.powervoice.module-info/1` JSON, checked against the `params` extension, replaces the
//!   generic parameter mapping.
//!
//! Threads follow CLAP's contract: every `[main-thread]` call happens on the sandbox's main
//! thread (the control loop that also loads the plugin), every `[audio-thread]` call on the
//! sandbox audio thread.

mod host;
mod instance;
mod library;
mod module_info;
mod params;
pub mod scan;
mod stream;

use std::path::Path;
use std::sync::Arc;

use vox_sandbox_ipc::protocol::ClapPluginRef;

use crate::backend::{PluginBackend, PluginInstance};

pub use instance::ClapInstance;

/// The `"clap"` backend.
pub struct ClapBackend;

impl PluginBackend for ClapBackend {
    fn name(&self) -> &'static str {
        "clap"
    }

    fn load(&self, plugin: &str) -> Result<Arc<dyn PluginInstance>, String> {
        let r = ClapPluginRef::parse(plugin)?;
        Ok(Arc::new(ClapInstance::load(Path::new(&r.path), &r.id)?))
    }
}

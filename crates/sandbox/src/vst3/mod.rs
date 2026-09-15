//! The **VST3 backend** (T-806, ADR-008 Amendment 6): format `"vst3"`, plugin reference =
//! [`Vst3PluginRef`] (the `.vst3` bundle and the processor's class id, JSON). Hosted through the
//! `vst3` crate (coupler-rs, MIT OR Apache-2.0), whose bindings are generated from the MIT
//! VST 3 SDK 3.8.0 headers (ADR-007 §6).
//!
//! - [`module`]: the bundle's binary for this platform, the per-OS entry (`ModuleEntry`/
//!   `ModuleExit`, `bundleEntry`/`bundleExit`, `InitDll`/`ExitDll`), `GetPluginFactory`, the
//!   factory's classes (`IPluginFactory2` when offered).
//! - [`host`]: the COM objects PowerVoice presents — `IHostApplication` (it allocates the
//!   `IMessage`/`IAttributeList` a component and its controller exchange through their
//!   connection points), `IComponentHandler` (begin/perform/end edit, restart flags), and the
//!   preallocated `IParameterChanges` queues of the audio thread.
//! - [`instance`]: one plugin as a [`PluginInstance`] — `IComponent` + `IAudioProcessor` +
//!   `IEditController` (separate, joined through connection points, or combined), bus
//!   arrangements with the mono shim, `setupProcessing`/`setActive`/`setProcessing`/`process`,
//!   sample-accurate parameter queues, latency and tail, framed component + controller state,
//!   the plugin's own parameter text.
//! - [`params`]: parameter info → the module API's schema (normalized / step-index domains).
//! - [`scan`]: `powervoice-sandbox --scan <bundle> --format vst3`.
//!
//! Threads follow the VST3 contract: every `[UI-thread]` call happens on the sandbox's main
//! thread (the control loop that loads the plugin), `setProcessing` and `process` on the sandbox
//! audio thread.

// VST3 enum constants are `c_uint` on unix and `c_int` on Windows (`DefaultEnumType`): a cast
// that is a no-op on one platform is needed on the other.
#![allow(clippy::unnecessary_cast)]

mod host;
mod instance;
mod module;
mod params;
pub mod scan;
mod stream;
mod uid;

use std::path::Path;
use std::sync::Arc;

use vox_sandbox_ipc::protocol::Vst3PluginRef;

use crate::backend::{PluginBackend, PluginInstance};

pub use instance::Vst3Instance;

/// The `"vst3"` backend.
pub struct Vst3Backend;

impl PluginBackend for Vst3Backend {
    fn name(&self) -> &'static str {
        "vst3"
    }

    fn load(&self, plugin: &str) -> Result<Arc<dyn PluginInstance>, String> {
        let r = Vst3PluginRef::parse(plugin)?;
        Ok(Arc::new(Vst3Instance::load(Path::new(&r.path), &r.cid)?))
    }
}

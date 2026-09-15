//! The in-process plugin backend seam (T-802): the test backend, CLAP (T-803) and VST3 (T-806);
//! LV2/JSFX come later behind the same trait.

use std::sync::Arc;

use vox_module_api::{ActivateConfig, ParamId};
use vox_sandbox_ipc::protocol::PluginInfo;
use vox_sandbox_ipc::{Chunk, WireEvent};

/// Plugin → host events one chunk may report at most (the audio loop's preallocated buffer;
/// [`PluginInstance::process`] must not push beyond its capacity).
pub const OUT_EVENT_CAPACITY: usize = 512;

/// What an activated plugin reports.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ActiveInfo {
    /// The plugin's own latency in samples.
    pub latency_samples: u32,
    /// Tail in samples; `None` = infinite.
    pub tail_samples: Option<u64>,
}

/// A plugin format host (`"test"`, later `"clap"`, `"vst3"`, …).
pub trait PluginBackend: Send + Sync {
    /// The name the host's `Load { backend }` request uses.
    fn name(&self) -> &'static str;
    /// Instantiates `plugin` (a backend-specific reference: path, id, options).
    fn load(&self, plugin: &str) -> Result<Arc<dyn PluginInstance>, String>;
}

/// One plugin instance. Shared between the sandbox's main thread (control requests) and its
/// audio thread (`process`), like a CLAP plugin's main-thread and audio-thread calls: the
/// backend synchronises internally. The sandbox never calls `set_param`/`load_state` while the
/// instance is active, nor `process` while it is inactive.
pub trait PluginInstance: Send + Sync {
    /// Name, schema and current values.
    fn info(&self) -> PluginInfo;
    /// \[main\] Activates for `config` (the segment's `max_block`, the host's rate and mode).
    fn activate(&self, config: &ActivateConfig) -> Result<ActiveInfo, String>;
    /// \[main\] Deactivates (after the audio thread stopped).
    fn deactivate(&self);
    /// \[audio\] Processes one chunk; plugin → host events go to `out_events` (never beyond
    /// its capacity: no allocation).
    fn process(&self, chunk: Chunk<'_>, out_events: &mut Vec<WireEvent>);
    /// \[main, inactive\] Sets a parameter.
    fn set_param(&self, id: ParamId, value: f64) -> Result<(), String>;
    /// \[main\] The plugin's state (also while active).
    fn save_state(&self) -> Result<Vec<u8>, String>;
    /// \[main, inactive\] Loads a state [`save_state`](Self::save_state) produced.
    fn load_state(&self, data: &[u8]) -> Result<(), String>;
    /// \[main\] Called every few milliseconds while the control loop is idle (T-803: CLAP
    /// `request_callback` → `on_main_thread`, `request_flush` while inactive).
    fn main_thread_idle(&self) {}
    /// \[audio\] The audio thread is about to stop serving (before `deactivate`; CLAP
    /// `stop_processing` belongs to the audio thread).
    fn audio_thread_stopping(&self) {}
    /// \[main\] The plugin's own display text for `value` (`None`: no text of its own).
    fn param_to_text(&self, _id: ParamId, _value: f64) -> Option<String> {
        None
    }
    /// \[main\] Parses `text` with the plugin's own parser (`None`: can't).
    fn text_to_param(&self, _id: ParamId, _text: &str) -> Option<f64> {
        None
    }
}

/// The backends this build of the sandbox links.
pub fn backends() -> Vec<Box<dyn PluginBackend>> {
    vec![
        Box::new(crate::test_backend::TestBackend),
        Box::new(crate::clap::ClapBackend),
        Box::new(crate::vst3::Vst3Backend),
    ]
}

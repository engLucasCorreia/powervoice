//! The in-process plugin backend seam (T-802): the test backend, CLAP (T-803), VST3 (T-806) and
//! LV2 (T-807) and JSFX (T-808).

use std::sync::Arc;

use vox_module_api::{ActivateConfig, ParamId};
use vox_sandbox_ipc::protocol::{ParamValue, PluginInfo};
use vox_sandbox_ipc::{Chunk, WireEvent};

use crate::gui::EditorHost;

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

/// What a plugin did on the main thread since the previous idle tick (T-901), reported to the
/// host by the sandbox.
#[derive(Debug, Default)]
pub struct MainThreadEvents {
    /// Parameters the plugin changed itself while **inactive** (its window): sent as a
    /// notification. While active, such changes are `process()` output events instead.
    pub params: Vec<ParamValue>,
    /// The plugin's state changed outside its parameters (CLAP `mark_dirty`, …).
    pub state_dirty: bool,
    /// A floating editor window closed on its own (the user closed it, or the plugin did).
    pub editor_closed: bool,
    /// An embedded editor asks for this size.
    pub editor_resize: Option<(u32, u32)>,
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
    /// `request_callback` → `on_main_thread`, `request_flush` while inactive; T-901: what the
    /// plugin's window did goes to `events`).
    fn main_thread_idle(&self, _events: &mut MainThreadEvents) {}
    /// \[main\] The plugin has an editor window this backend can show (T-901).
    fn has_editor(&self) -> bool {
        false
    }
    /// \[main\] Opens the plugin's editor (T-901): embedded in a top-level window `host` creates
    /// ([`EditorHost::create_window`]), or as the plugin's own floating window. Returns its size.
    fn open_editor(&self, _host: &mut EditorHost<'_>) -> Result<(u32, u32), String> {
        Err("the plugin has no window of its own".into())
    }
    /// \[main\] Destroys the editor (the sandbox then destroys its window).
    fn close_editor(&self) {}
    /// \[main\] The user resized an embedded editor's window to `width`×`height`: the size the
    /// plugin accepts, when it differs (`None`: as is).
    fn editor_resized(&self, _width: u32, _height: u32) -> Option<(u32, u32)> {
        None
    }
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
        Box::new(crate::lv2::Lv2Backend),
        Box::new(crate::jsfx::JsfxBackend),
    ]
}

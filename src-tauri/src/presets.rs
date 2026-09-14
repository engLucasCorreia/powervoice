//! T-406: the app's preset stores, rooted at [`settings::presets_dir`]. Thin: all storage,
//! validation and factory-preset logic lives in `vox_presets`; this is just the Tauri-managed
//! handle the IPC commands share.

use vox_presets::{ModulePresetStore, RackPresetStore};

use crate::settings::presets_dir;

/// The app's module- and rack-preset stores (Tauri-managed state).
#[derive(Clone)]
pub struct PresetStores {
    pub modules: std::sync::Arc<ModulePresetStore>,
    pub racks: std::sync::Arc<RackPresetStore>,
}

impl PresetStores {
    /// Stores rooted at the OS config dir's `presets/modules` and `presets/racks`
    /// (SPEC-001-style app config dir, T-406). Doesn't touch the filesystem until a command
    /// uses them.
    pub fn load_default() -> Self {
        let root = presets_dir();
        Self {
            modules: std::sync::Arc::new(ModulePresetStore::new(root.join("modules"))),
            racks: std::sync::Arc::new(RackPresetStore::new(root.join("racks"))),
        }
    }
}

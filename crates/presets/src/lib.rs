//! Module & rack preset storage and validation (T-406, ADR-005 §2/§10/§12, SPEC-012 §2.7).
//!
//! Presets are versioned JSON files under the app config dir (`presets/modules/<module-id>/
//! <name>.json`, `presets/racks/<name>.json`), written atomically. Applying a loaded preset to a
//! live rack is `vox_rack::RackHost`'s job (`apply_module_preset`/`reset_to_default`); this crate
//! only knows about files, names and the built-in factory rack presets.

mod atomic;
mod error;
mod export;
mod factory_rack;
mod module_store;
mod name;
mod rack_store;

pub use error::PresetError;
pub use export::{ExportedPreset, export_to_file, import_from_file};
pub use factory_rack::{FactoryRackPreset, factory_rack_presets};
pub use module_store::{ModulePresetStore, StoredModulePreset};
pub use name::{sanitize_module_id, sanitize_preset_name};
pub use rack_store::{RackPresetStore, StoredRackPreset};

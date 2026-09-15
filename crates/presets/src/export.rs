//! Export/import a single preset as a standalone `.json` file (H-22, SPEC-012 §2.7 follow-up): a
//! portable form of one [`crate::StoredModulePreset`]/[`crate::StoredRackPreset`], self-describing
//! (`kind`) so importing doesn't need to be told which kind of file it's opening.
//!
//! The destination/source path is wherever the user's native file dialog pointed — not the
//! presets directory — so this module never sanitizes *it*. What it does still sanitize is the
//! *name* an imported file claims for itself: [`import_from_file`] returns it verbatim (even a
//! traversal string like `"evil/../../etc"`), and the caller is expected to hand it straight to
//! [`crate::ModulePresetStore::save`]/[`crate::RackPresetStore::save`] — exactly like a name typed
//! into the "Save as…" field — which sanitizes it the normal way before it ever becomes a file
//! name. Nothing in this module writes into a `ModulePresetStore`/`RackPresetStore` root itself.

use std::path::Path;

use serde::{Deserialize, Serialize};
use vox_module_api::ModuleState;
use vox_rack::RackModel;

use crate::atomic::write_atomic;
use crate::error::PresetError;
use crate::module_store::CURRENT_MODULE_PRESET_FORMAT_VERSION;
use crate::rack_store::CURRENT_RACK_PRESET_FORMAT_VERSION;

/// One exported preset file's shape; `kind` (the serde tag) says which. Mirrors
/// [`crate::StoredModulePreset`]/[`crate::StoredRackPreset`] plus, for a module preset, the
/// `module_id` that file's own directory would otherwise have implied.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ExportedPreset {
    Module {
        format_version: u32,
        module_id: String,
        name: String,
        state: ModuleState,
    },
    Rack {
        format_version: u32,
        name: String,
        rack: RackModel,
    },
}

impl ExportedPreset {
    /// Builds a module preset export at the current format version.
    pub fn module(
        module_id: impl Into<String>,
        name: impl Into<String>,
        state: ModuleState,
    ) -> Self {
        Self::Module {
            format_version: CURRENT_MODULE_PRESET_FORMAT_VERSION,
            module_id: module_id.into(),
            name: name.into(),
            state,
        }
    }

    /// Builds a rack preset export at the current format version.
    pub fn rack(name: impl Into<String>, rack: RackModel) -> Self {
        Self::Rack {
            format_version: CURRENT_RACK_PRESET_FORMAT_VERSION,
            name: name.into(),
            rack,
        }
    }

    fn name(&self) -> &str {
        match self {
            Self::Module { name, .. } | Self::Rack { name, .. } => name,
        }
    }
}

/// Writes `preset` to `dest` (any path the caller was given — typically the user's native save
/// dialog — never the presets directory, so never sanitized).
pub fn export_to_file(preset: &ExportedPreset, dest: &Path) -> Result<(), PresetError> {
    let json = serde_json::to_vec_pretty(preset).map_err(|e| PresetError::Corrupt {
        name: preset.name().to_owned(),
        message: e.to_string(),
    })?;
    write_atomic(dest, &json)
}

/// Parses an exported preset file's bytes (SPEC-012 §2.7 import): malformed JSON is
/// [`PresetError::Corrupt`], a `format_version` newer than this build supports is
/// [`PresetError::TooNew`]. The `name` (and, for a module preset, `module_id`) inside is
/// returned exactly as read — see the module docs for why that's safe.
pub fn import_from_file(bytes: &[u8]) -> Result<ExportedPreset, PresetError> {
    let preset: ExportedPreset =
        serde_json::from_slice(bytes).map_err(|e| PresetError::Corrupt {
            name: "imported preset".to_owned(),
            message: e.to_string(),
        })?;
    let (version, current) = match &preset {
        ExportedPreset::Module { format_version, .. } => {
            (*format_version, CURRENT_MODULE_PRESET_FORMAT_VERSION)
        }
        ExportedPreset::Rack { format_version, .. } => {
            (*format_version, CURRENT_RACK_PRESET_FORMAT_VERSION)
        }
    };
    if version > current {
        return Err(PresetError::TooNew {
            name: preset.name().to_owned(),
        });
    }
    Ok(preset)
}

#[cfg(test)]
#[allow(clippy::float_cmp)] // exact values round-tripped through JSON
mod tests {
    use std::collections::BTreeMap;

    use vox_module_api::{ModuleRef, Version};
    use vox_rack::SlotModel;

    use super::*;
    use crate::{ModulePresetStore, RackPresetStore};

    fn tmp_dir(tag: &str) -> std::path::PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "vox-presets-export-{tag}-{}-{nanos}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn module_state(gain_db: f64) -> ModuleState {
        ModuleState {
            format_version: 1,
            params: BTreeMap::from([("gain_db".to_owned(), gain_db)]),
            blob: None,
        }
    }

    #[test]
    fn a_module_preset_round_trips_through_a_file() {
        let dir = tmp_dir("module-roundtrip");
        let path = dir.join("Mine.json");
        let exported = ExportedPreset::module("org.powervoice.gain", "Mine", module_state(-6.0));
        export_to_file(&exported, &path).unwrap();

        let bytes = std::fs::read(&path).unwrap();
        let imported = import_from_file(&bytes).unwrap();
        assert_eq!(imported, exported);
    }

    #[test]
    fn a_rack_preset_round_trips_through_a_file_including_an_unknown_module_id() {
        // ADR-005 §2: a rack preset naming a module this build doesn't have still round-trips
        // verbatim (it becomes a placeholder slot only when actually loaded).
        let dir = tmp_dir("rack-roundtrip");
        let path = dir.join("Mine.json");
        let rack = RackModel {
            slots: vec![SlotModel::new(
                &ModuleRef {
                    id: "org.example.future-plugin".into(),
                    version: Version::new(2, 0, 0),
                },
                false,
                &ModuleState::new(1),
            )],
        };
        let exported = ExportedPreset::rack("Mine", rack);
        export_to_file(&exported, &path).unwrap();

        let bytes = std::fs::read(&path).unwrap();
        let imported = import_from_file(&bytes).unwrap();
        assert_eq!(imported, exported);
    }

    #[test]
    fn malformed_json_is_corrupt_not_a_panic() {
        assert!(matches!(
            import_from_file(b"{ not json"),
            Err(PresetError::Corrupt { .. })
        ));
    }

    #[test]
    fn a_future_format_version_is_too_new_not_corrupt() {
        let bytes = serde_json::to_vec(&serde_json::json!({
            "kind": "module",
            "format_version": CURRENT_MODULE_PRESET_FORMAT_VERSION + 1,
            "module_id": "org.powervoice.gain",
            "name": "Future",
            "state": { "format_version": 1, "params": {} },
        }))
        .unwrap();
        assert!(matches!(
            import_from_file(&bytes),
            Err(PresetError::TooNew { .. })
        ));
    }

    #[test]
    fn an_unrecognized_shape_is_corrupt() {
        let bytes = serde_json::to_vec(&serde_json::json!({ "hello": "world" })).unwrap();
        assert!(matches!(
            import_from_file(&bytes),
            Err(PresetError::Corrupt { .. })
        ));
    }

    #[test]
    fn a_name_that_looks_like_path_traversal_round_trips_verbatim_here_and_is_sanitized_by_the_store()
     {
        // This module never touches a presets directory, so it never sanitizes `name` itself
        // (see the module docs) — but the store the caller ultimately saves into still does.
        let dir = tmp_dir("traversal-file");
        let path = dir.join("evil.json");
        let exported =
            ExportedPreset::module("org.powervoice.gain", "evil/../../etc", module_state(0.0));
        export_to_file(&exported, &path).unwrap();

        let bytes = std::fs::read(&path).unwrap();
        let imported = import_from_file(&bytes).unwrap();
        let ExportedPreset::Module {
            module_id,
            name,
            state,
            ..
        } = imported
        else {
            panic!("expected a module preset");
        };
        assert_eq!(
            name, "evil/../../etc",
            "returned verbatim, not pre-sanitized"
        );

        let store_root = tmp_dir("traversal-store");
        let store = ModulePresetStore::new(store_root.clone());
        let saved = store.save(&module_id, &name, &state).unwrap();
        // Whatever `sanitize_preset_name` turns it into, it stayed a single path component
        // inside `module_id`'s own directory — nothing was written above `store_root`.
        assert!(!saved.contains('/') && !saved.contains('\\'));
        assert!(
            !store_root
                .parent()
                .unwrap()
                .join(format!("{saved}.json"))
                .exists()
        );
        assert_eq!(store.list(&module_id).unwrap(), vec![saved]);
    }

    #[test]
    fn a_rack_preset_name_that_looks_like_path_traversal_is_sanitized_by_the_store() {
        let dir = tmp_dir("traversal-rack-file");
        let path = dir.join("evil.json");
        let exported = ExportedPreset::rack("evil/../../etc", RackModel { slots: vec![] });
        export_to_file(&exported, &path).unwrap();

        let bytes = std::fs::read(&path).unwrap();
        let imported = import_from_file(&bytes).unwrap();
        let ExportedPreset::Rack { name, rack, .. } = imported else {
            panic!("expected a rack preset");
        };

        let store_root = tmp_dir("traversal-rack-store");
        let store = RackPresetStore::new(store_root.clone());
        let saved = store.save(&name, &rack).unwrap();
        assert!(!saved.contains('/') && !saved.contains('\\'));
        assert!(
            !store_root
                .parent()
                .unwrap()
                .join(format!("{saved}.json"))
                .exists()
        );
        assert_eq!(store.list().unwrap(), vec![saved]);
    }
}

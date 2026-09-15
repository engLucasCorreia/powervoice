//! User rack presets (T-406, SPEC-012 "the rack-preset menu"): `presets/racks/<name>.json`.
//!
//! Loading a stored rack preset is `RackModel::from_json` + `RackHost::load_model` (already
//! generic over any `RackModel`, ADR-005 §2 "Resolving a `ModuleRef`"): an unknown module id
//! becomes a placeholder slot with a notice exactly like opening a document whose sidecar names
//! a module that isn't installed — this store never needs to special-case that itself.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use vox_rack::RackModel;

use crate::atomic::{list_json_names, write_atomic};
use crate::error::PresetError;
use crate::name::sanitize_preset_name;

/// Current on-disk schema version of a stored rack preset file.
pub const CURRENT_RACK_PRESET_FORMAT_VERSION: u32 = 1;

/// One rack preset as stored on disk: an ordered list of slots (ADR-005 §10 "a rack preset is an
/// ordered list of slots"), in the sidecar's slot schema. `extra` preserves unknown top-level
/// fields written by a newer build.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StoredRackPreset {
    pub format_version: u32,
    pub name: String,
    pub rack: RackModel,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

fn parse(bytes: &[u8], name: &str) -> Result<StoredRackPreset, PresetError> {
    let stored: StoredRackPreset =
        serde_json::from_slice(bytes).map_err(|e| PresetError::Corrupt {
            name: name.to_owned(),
            message: e.to_string(),
        })?;
    if stored.format_version > CURRENT_RACK_PRESET_FORMAT_VERSION {
        return Err(PresetError::TooNew {
            name: name.to_owned(),
        });
    }
    Ok(stored)
}

fn not_found_or_io(name: &str, err: std::io::Error) -> PresetError {
    if err.kind() == std::io::ErrorKind::NotFound {
        PresetError::NotFound(name.to_owned())
    } else {
        PresetError::Io(err)
    }
}

fn to_json(stored: &StoredRackPreset) -> Result<Vec<u8>, PresetError> {
    serde_json::to_vec_pretty(stored).map_err(|e| PresetError::Corrupt {
        name: stored.name.clone(),
        message: e.to_string(),
    })
}

/// Reads, writes, renames and deletes user-saved rack presets under one root directory
/// (`<app config dir>/presets/racks`). Factory rack presets ([`crate::factory_rack_presets`])
/// live in code, not here.
pub struct RackPresetStore {
    root: PathBuf,
}

impl RackPresetStore {
    /// A store rooted at `root` (typically `<presets dir>/racks`). Doesn't touch the filesystem
    /// until a method is called.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    fn path(&self, name: &str) -> Result<PathBuf, PresetError> {
        Ok(self
            .root
            .join(format!("{}.json", sanitize_preset_name(name)?)))
    }

    /// User rack preset names, sorted.
    pub fn list(&self) -> Result<Vec<String>, PresetError> {
        list_json_names(&self.root)
    }

    /// Saves `rack` as a new preset named `name`. Fails with [`PresetError::AlreadyExists`] if
    /// one exists — call [`Self::delete`] first to overwrite.
    pub fn save(&self, name: &str, rack: &RackModel) -> Result<String, PresetError> {
        let sanitized = sanitize_preset_name(name)?;
        let path = self.root.join(format!("{sanitized}.json"));
        if path.exists() {
            return Err(PresetError::AlreadyExists(sanitized));
        }
        let stored = StoredRackPreset {
            format_version: CURRENT_RACK_PRESET_FORMAT_VERSION,
            name: sanitized.clone(),
            rack: rack.clone(),
            extra: Map::new(),
        };
        write_atomic(&path, &to_json(&stored)?)?;
        Ok(sanitized)
    }

    /// Loads the rack model saved as `name`.
    pub fn load(&self, name: &str) -> Result<RackModel, PresetError> {
        let path = self.path(name)?;
        let bytes = std::fs::read(&path).map_err(|e| not_found_or_io(name, e))?;
        Ok(parse(&bytes, name)?.rack)
    }

    /// Renames a preset (also updates the stored `name` field), returning the sanitized new
    /// name. Fails if `old` doesn't exist or `new` already does.
    pub fn rename(&self, old: &str, new: &str) -> Result<String, PresetError> {
        let old_path = self.path(old)?;
        let bytes = std::fs::read(&old_path).map_err(|e| not_found_or_io(old, e))?;
        let mut stored = parse(&bytes, old)?;
        let new_sanitized = sanitize_preset_name(new)?;
        let new_path = self.root.join(format!("{new_sanitized}.json"));
        if new_path.exists() {
            return Err(PresetError::AlreadyExists(new_sanitized));
        }
        stored.name = new_sanitized.clone();
        write_atomic(&new_path, &to_json(&stored)?)?;
        std::fs::remove_file(&old_path)?;
        Ok(new_sanitized)
    }

    /// Deletes a user rack preset. Fails with [`PresetError::NotFound`] if it doesn't exist.
    pub fn delete(&self, name: &str) -> Result<(), PresetError> {
        let path = self.path(name)?;
        std::fs::remove_file(&path).map_err(|e| not_found_or_io(name, e))
    }
}

#[cfg(test)]
mod tests {
    use vox_module_api::{ModuleRef, ModuleState, Version};
    use vox_rack::SlotModel;

    use super::*;

    fn tmp_dir(tag: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "vox-presets-rack-store-{tag}-{}-{nanos}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn gain_slot(db: f64) -> SlotModel {
        SlotModel::new(
            &ModuleRef {
                id: "org.powervoice.gain".into(),
                version: Version::new(1, 0, 0),
            },
            false,
            &ModuleState {
                format_version: 1,
                params: [("gain_db".to_owned(), db)].into_iter().collect(),
                blob: None,
            },
        )
    }

    #[test]
    fn save_then_load_round_trips_the_whole_chain() {
        let store = RackPresetStore::new(tmp_dir("roundtrip"));
        let rack = RackModel {
            slots: vec![gain_slot(-3.0), gain_slot(6.0)],
        };
        store.save("Podcast voice", &rack).unwrap();
        let loaded = store.load("Podcast voice").unwrap();
        assert_eq!(loaded, rack);
    }

    /// T-810: a slot carrying a state blob (an external plugin's chunk, or a noise print) round
    /// trips byte-exact as part of the whole rack, not just a lone module preset (T-406 covers
    /// that; this closes the same gap for `RackPresetStore`, ADR-005 §10 "a rack preset is an
    /// ordered list of slots").
    #[test]
    fn save_then_load_round_trips_a_slot_with_a_blob_byte_exact() {
        let store = RackPresetStore::new(tmp_dir("roundtrip-blob"));
        let mut with_blob = gain_slot(-3.0);
        with_blob.state = serde_json::to_value(ModuleState {
            format_version: 1,
            params: [("gain_db".to_owned(), -3.0)].into_iter().collect(),
            blob: Some(vec![0, 1, 2, 250, 255, 0, 128]),
        })
        .unwrap();
        let rack = RackModel {
            slots: vec![with_blob, gain_slot(6.0)],
        };
        store.save("With print", &rack).unwrap();
        let loaded = store.load("With print").unwrap();
        assert_eq!(loaded, rack);
        let loaded_state: ModuleState =
            serde_json::from_value(loaded.slots[0].state.clone()).unwrap();
        assert_eq!(
            loaded_state.blob,
            Some(vec![0, 1, 2, 250, 255, 0, 128]),
            "blob bytes must survive base64 round trip exactly"
        );
    }

    #[test]
    fn list_rename_delete_behave_like_the_module_store() {
        let store = RackPresetStore::new(tmp_dir("crud"));
        let rack = RackModel {
            slots: vec![gain_slot(0.0)],
        };
        store.save("B", &rack).unwrap();
        store.save("A", &rack).unwrap();
        assert_eq!(store.list().unwrap(), vec!["A".to_owned(), "B".to_owned()]);

        let renamed = store.rename("A", "A2").unwrap();
        assert_eq!(renamed, "A2");
        assert!(matches!(store.load("A"), Err(PresetError::NotFound(_))));
        assert_eq!(store.list().unwrap(), vec!["A2".to_owned(), "B".to_owned()]);

        store.delete("B").unwrap();
        assert_eq!(store.list().unwrap(), vec!["A2".to_owned()]);
        assert!(matches!(store.delete("B"), Err(PresetError::NotFound(_))));
    }

    #[test]
    fn save_twice_fails_with_already_exists() {
        let store = RackPresetStore::new(tmp_dir("dup"));
        let rack = RackModel { slots: vec![] };
        store.save("Mine", &rack).unwrap();
        assert!(matches!(
            store.save("Mine", &rack),
            Err(PresetError::AlreadyExists(_))
        ));
    }

    #[test]
    fn a_rack_preset_with_an_unknown_module_id_round_trips_verbatim() {
        // ADR-005 §2: an unresolvable module ref round-trips verbatim (a rack preset saved by a
        // build with a module this one doesn't have installed still loads as a placeholder,
        // never data loss).
        let store = RackPresetStore::new(tmp_dir("unknown-module"));
        let rack = RackModel {
            slots: vec![SlotModel::new(
                &ModuleRef {
                    id: "org.example.some-future-plugin".into(),
                    version: Version::new(2, 0, 0),
                },
                false,
                &ModuleState::new(1),
            )],
        };
        store.save("Has an unknown module", &rack).unwrap();
        let loaded = store.load("Has an unknown module").unwrap();
        assert_eq!(loaded, rack);
        assert_eq!(
            loaded.slots[0].module,
            "org.example.some-future-plugin@2.0.0"
        );
    }
}

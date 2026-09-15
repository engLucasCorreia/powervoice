//! User module presets (T-406, SPEC-012 §2.7): `presets/modules/<module-id>/<name>.json`.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use vox_module_api::ModuleState;

use crate::atomic::{list_json_names, write_atomic};
use crate::error::PresetError;
use crate::name::{sanitize_module_id, sanitize_preset_name};

/// Current on-disk schema version of a stored module preset file.
pub const CURRENT_MODULE_PRESET_FORMAT_VERSION: u32 = 1;

/// One module preset as stored on disk (ADR-005 §10 "a module preset is `{ module, name,
/// state }`" — `module` is implicit in the file's directory here). `extra` preserves unknown
/// fields written by a newer build (SPEC-004/ADR-004's "unknown fields preserved" convention).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StoredModulePreset {
    pub format_version: u32,
    pub name: String,
    pub state: ModuleState,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

fn parse(bytes: &[u8], name: &str) -> Result<StoredModulePreset, PresetError> {
    let stored: StoredModulePreset =
        serde_json::from_slice(bytes).map_err(|e| PresetError::Corrupt {
            name: name.to_owned(),
            message: e.to_string(),
        })?;
    if stored.format_version > CURRENT_MODULE_PRESET_FORMAT_VERSION {
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

/// Reads, writes, renames and deletes user-saved module presets under one root directory
/// (`<app config dir>/presets/modules`), one subdirectory per module id. Factory presets
/// (`ModuleFactory::presets`, ADR-005 §2) live in code, not here — callers merge the two lists
/// for display (e.g. the rack slot's preset menu).
pub struct ModulePresetStore {
    root: PathBuf,
}

impl ModulePresetStore {
    /// A store rooted at `root` (typically `<presets dir>/modules`). Doesn't touch the
    /// filesystem until a method is called.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    fn module_dir(&self, module_id: &str) -> Result<PathBuf, PresetError> {
        Ok(self.root.join(sanitize_module_id(module_id)?))
    }

    fn path(&self, module_id: &str, name: &str) -> Result<PathBuf, PresetError> {
        Ok(self
            .module_dir(module_id)?
            .join(format!("{}.json", sanitize_preset_name(name)?)))
    }

    /// User preset names for `module_id`, sorted. Empty if none were ever saved.
    pub fn list(&self, module_id: &str) -> Result<Vec<String>, PresetError> {
        list_json_names(&self.module_dir(module_id)?)
    }

    /// Saves `state` as a new preset named `name`. Fails with
    /// [`PresetError::AlreadyExists`] if one exists — call [`Self::delete`] first to overwrite
    /// (an explicit choice, not a silent one: presets are named things a user chose to keep).
    pub fn save(
        &self,
        module_id: &str,
        name: &str,
        state: &ModuleState,
    ) -> Result<String, PresetError> {
        let sanitized = sanitize_preset_name(name)?;
        let path = self
            .module_dir(module_id)?
            .join(format!("{sanitized}.json"));
        if path.exists() {
            return Err(PresetError::AlreadyExists(sanitized));
        }
        write_stored(&path, &sanitized, state)?;
        Ok(sanitized)
    }

    /// Loads the module state saved as `name`.
    pub fn load(&self, module_id: &str, name: &str) -> Result<ModuleState, PresetError> {
        let path = self.path(module_id, name)?;
        let bytes = std::fs::read(&path).map_err(|e| not_found_or_io(name, e))?;
        Ok(parse(&bytes, name)?.state)
    }

    /// Renames a preset (also updates the stored `name` field), returning the sanitized new
    /// name. Fails if `old` doesn't exist or `new` already does.
    pub fn rename(&self, module_id: &str, old: &str, new: &str) -> Result<String, PresetError> {
        let old_path = self.path(module_id, old)?;
        let bytes = std::fs::read(&old_path).map_err(|e| not_found_or_io(old, e))?;
        let mut stored = parse(&bytes, old)?;
        let new_sanitized = sanitize_preset_name(new)?;
        let new_path = self
            .module_dir(module_id)?
            .join(format!("{new_sanitized}.json"));
        if new_path.exists() {
            return Err(PresetError::AlreadyExists(new_sanitized));
        }
        stored.name = new_sanitized.clone();
        write_atomic(&new_path, &to_json(&stored)?)?;
        std::fs::remove_file(&old_path)?;
        Ok(new_sanitized)
    }

    /// Deletes a user preset. Fails with [`PresetError::NotFound`] if it doesn't exist.
    pub fn delete(&self, module_id: &str, name: &str) -> Result<(), PresetError> {
        let path = self.path(module_id, name)?;
        std::fs::remove_file(&path).map_err(|e| not_found_or_io(name, e))
    }
}

fn to_json(stored: &StoredModulePreset) -> Result<Vec<u8>, PresetError> {
    serde_json::to_vec_pretty(stored).map_err(|e| PresetError::Corrupt {
        name: stored.name.clone(),
        message: e.to_string(),
    })
}

fn write_stored(path: &Path, name: &str, state: &ModuleState) -> Result<(), PresetError> {
    let stored = StoredModulePreset {
        format_version: CURRENT_MODULE_PRESET_FORMAT_VERSION,
        name: name.to_owned(),
        state: state.clone(),
        extra: Map::new(),
    };
    write_atomic(path, &to_json(&stored)?)
}

#[cfg(test)]
#[allow(clippy::float_cmp)] // exact values round-tripped through JSON
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    fn tmp_dir(tag: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "vox-presets-module-store-{tag}-{}-{nanos}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn state(params: &[(&str, f64)]) -> ModuleState {
        ModuleState {
            format_version: 1,
            params: params
                .iter()
                .map(|(k, v)| ((*k).to_owned(), *v))
                .collect::<BTreeMap<_, _>>(),
            blob: None,
        }
    }

    #[test]
    fn save_then_load_round_trips_params_and_blob_bit_exactly() {
        let store = ModulePresetStore::new(tmp_dir("roundtrip"));
        let mut s = state(&[("gain_db", -6.5), ("weird", 1.0 / 3.0)]);
        s.blob = Some(vec![0, 1, 2, 250]);
        store.save("org.powervoice.gain", "My preset", &s).unwrap();
        let loaded = store.load("org.powervoice.gain", "My preset").unwrap();
        assert_eq!(loaded.params["gain_db"].to_bits(), (-6.5f64).to_bits());
        assert_eq!(
            loaded.params["weird"].to_bits(),
            (1.0f64 / 3.0).to_bits(),
            "f64 must round-trip exactly through JSON (serde_json float_roundtrip)"
        );
        assert_eq!(loaded.blob, s.blob);
    }

    #[test]
    fn save_twice_without_delete_fails_with_already_exists() {
        let store = ModulePresetStore::new(tmp_dir("dup"));
        store
            .save("org.powervoice.gain", "Mine", &state(&[]))
            .unwrap();
        assert!(matches!(
            store.save("org.powervoice.gain", "Mine", &state(&[])),
            Err(PresetError::AlreadyExists(_))
        ));
    }

    #[test]
    fn load_missing_preset_is_not_found() {
        let store = ModulePresetStore::new(tmp_dir("missing"));
        assert!(matches!(
            store.load("org.powervoice.gain", "Nope"),
            Err(PresetError::NotFound(_))
        ));
    }

    #[test]
    fn list_is_sorted_and_scoped_to_the_module_id() {
        let store = ModulePresetStore::new(tmp_dir("list"));
        store.save("org.powervoice.gain", "B", &state(&[])).unwrap();
        store.save("org.powervoice.gain", "A", &state(&[])).unwrap();
        store
            .save("org.powervoice.dynamics", "Other module", &state(&[]))
            .unwrap();
        assert_eq!(
            store.list("org.powervoice.gain").unwrap(),
            vec!["A".to_owned(), "B".to_owned()]
        );
        assert_eq!(
            store.list("org.powervoice.dynamics").unwrap(),
            vec!["Other module".to_owned()]
        );
        assert_eq!(
            store.list("org.powervoice.noise-gate").unwrap(),
            Vec::<String>::new()
        );
    }

    #[test]
    fn rename_moves_the_file_and_updates_the_stored_name() {
        let store = ModulePresetStore::new(tmp_dir("rename"));
        store
            .save(
                "org.powervoice.gain",
                "Old name",
                &state(&[("gain_db", 3.0)]),
            )
            .unwrap();
        let renamed = store
            .rename("org.powervoice.gain", "Old name", "New name")
            .unwrap();
        assert_eq!(renamed, "New name");
        assert!(matches!(
            store.load("org.powervoice.gain", "Old name"),
            Err(PresetError::NotFound(_))
        ));
        assert_eq!(
            store
                .load("org.powervoice.gain", "New name")
                .unwrap()
                .params["gain_db"],
            3.0
        );
        assert_eq!(
            store.list("org.powervoice.gain").unwrap(),
            vec!["New name".to_owned()]
        );
    }

    #[test]
    fn rename_onto_an_existing_name_fails_and_keeps_the_original() {
        let store = ModulePresetStore::new(tmp_dir("rename-conflict"));
        store.save("org.powervoice.gain", "A", &state(&[])).unwrap();
        store.save("org.powervoice.gain", "B", &state(&[])).unwrap();
        assert!(matches!(
            store.rename("org.powervoice.gain", "A", "B"),
            Err(PresetError::AlreadyExists(_))
        ));
        assert!(store.load("org.powervoice.gain", "A").is_ok());
    }

    #[test]
    fn delete_removes_the_preset_and_is_not_found_afterwards() {
        let store = ModulePresetStore::new(tmp_dir("delete"));
        store
            .save("org.powervoice.gain", "Gone soon", &state(&[]))
            .unwrap();
        store.delete("org.powervoice.gain", "Gone soon").unwrap();
        assert!(matches!(
            store.load("org.powervoice.gain", "Gone soon"),
            Err(PresetError::NotFound(_))
        ));
    }

    #[test]
    fn delete_missing_preset_is_not_found() {
        let store = ModulePresetStore::new(tmp_dir("delete-missing"));
        assert!(matches!(
            store.delete("org.powervoice.gain", "Nope"),
            Err(PresetError::NotFound(_))
        ));
    }

    #[test]
    fn a_path_traversal_name_never_writes_outside_the_module_directory() {
        let root = tmp_dir("traversal");
        let store = ModulePresetStore::new(root.clone());
        store
            .save("org.powervoice.gain", "../../evil", &state(&[]))
            .unwrap();
        // Nothing was written above the module directory.
        assert!(!root.parent().unwrap().join("evil.json").exists());
        let gain_dir = root.join("org.powervoice.gain");
        assert!(gain_dir.is_dir());
        let entries: Vec<_> = std::fs::read_dir(&gain_dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(entries.len(), 1, "{entries:?}");
    }

    /// H-33: `sanitize_preset_name("../../evil")` used to produce a name starting with `.`
    /// (`".._.._evil"`), which saved fine but then `list_json_names` hid it as if it were an
    /// atomic-write temp file — the preset existed on disk but never showed up in the UI's preset
    /// menu. Prove the full round trip: save a traversal-like name, see it in `list`, `load` it
    /// back by the name `save` returned.
    #[test]
    fn traversal_like_name_round_trips_through_save_list_and_load() {
        let store = ModulePresetStore::new(tmp_dir("traversal-round-trip"));
        let saved_name = store
            .save(
                "org.powervoice.gain",
                "../../evil",
                &state(&[("gain_db", 2.0)]),
            )
            .unwrap();
        assert!(
            !saved_name.starts_with('.'),
            "sanitized name {saved_name:?} must not start with '.'"
        );
        assert_eq!(
            store.list("org.powervoice.gain").unwrap(),
            vec![saved_name.clone()]
        );
        let loaded = store.load("org.powervoice.gain", &saved_name).unwrap();
        assert_eq!(loaded.params["gain_db"], 2.0);
    }

    #[test]
    fn corrupt_json_is_reported_as_corrupt_not_a_panic() {
        let root = tmp_dir("corrupt");
        std::fs::create_dir_all(root.join("org.powervoice.gain")).unwrap();
        std::fs::write(
            root.join("org.powervoice.gain").join("Bad.json"),
            b"{ not json",
        )
        .unwrap();
        let store = ModulePresetStore::new(root);
        assert!(matches!(
            store.load("org.powervoice.gain", "Bad"),
            Err(PresetError::Corrupt { .. })
        ));
    }

    #[test]
    fn a_future_format_version_is_too_new_not_corrupt() {
        let root = tmp_dir("too-new");
        std::fs::create_dir_all(root.join("org.powervoice.gain")).unwrap();
        std::fs::write(
            root.join("org.powervoice.gain").join("Future.json"),
            format!(
                r#"{{"format_version":{},"name":"Future","state":{{"format_version":1,"params":{{}}}}}}"#,
                CURRENT_MODULE_PRESET_FORMAT_VERSION + 1
            ),
        )
        .unwrap();
        let store = ModulePresetStore::new(root);
        assert!(matches!(
            store.load("org.powervoice.gain", "Future"),
            Err(PresetError::TooNew { .. })
        ));
    }

    #[test]
    fn an_atomic_write_crash_leftover_does_not_appear_in_a_listing() {
        let root = tmp_dir("crash-listing");
        let store = ModulePresetStore::new(root.clone());
        store
            .save("org.powervoice.gain", "Real", &state(&[]))
            .unwrap();
        std::fs::write(
            root.join("org.powervoice.gain")
                .join(format!(".Crashed.json.tmp-{}", std::process::id())),
            b"{ incomplete",
        )
        .unwrap();
        assert_eq!(
            store.list("org.powervoice.gain").unwrap(),
            vec!["Real".to_owned()]
        );
    }
}

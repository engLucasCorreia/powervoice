//! Module & rack preset commands (T-406, SPEC-012 §2.7, ADR-005 §2/§10/§12). Thin: storage,
//! validation and factory-preset content live in `vox_presets`; applying a loaded preset to the
//! live rack forwards a `vox_engine::RackCommand` exactly like every other rack command
//! (`rack_commands::apply`).

use std::path::Path;

use tauri::State;
use vox_engine::RackCommand;
use vox_presets::{
    ExportedPreset, ModulePresetStore, PresetError, RackPresetStore, export_to_file,
    import_from_file,
};
use vox_rack::{ModuleState, RackModel};

use crate::audio::AudioEngine;
use crate::document::DocumentService;
use crate::ipc::error::{IpcError, IpcErrorCode};
use crate::ipc::preset_dto::{
    ModulePresetImportedDto, PresetEntryDto, PresetRefDto, preset_ipc_error,
};
use crate::ipc::rack_commands::apply;
use crate::ipc::rack_dto::{LocalizedTextDto, RackStateDto, rack_ipc_error};
use crate::presets::PresetStores;

fn join_blocking_err(e: impl std::fmt::Display) -> IpcError {
    IpcError::internal(e.to_string())
}

fn user_entry(name: String) -> PresetEntryDto {
    PresetEntryDto {
        key: name.clone(),
        name: LocalizedTextDto {
            text: name,
            key: None,
        },
        is_factory: false,
    }
}

// --- Module presets --------------------------------------------------------------------------

/// Factory presets first — the built-in's own (`ModuleFactory::presets`, ADR-005 §2), then a
/// `.voxmod` package's `presets/*.vopreset.json` (H-44, ADR-006 §3/§7 step 5; empty unless
/// `module_id` was installed from a package that shipped some) — then user-saved ones
/// (alphabetical) — the slot menu's preset list (SPEC-012 §2.7).
#[tauri::command]
pub async fn module_presets_list(
    engine: State<'_, AudioEngine>,
    presets: State<'_, PresetStores>,
    module_id: String,
) -> Result<Vec<PresetEntryDto>, IpcError> {
    let handle = engine.handle().clone();
    let store = presets.modules.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let mut out: Vec<PresetEntryDto> = handle
            .rack_module_presets(&module_id)
            .iter()
            .chain(crate::plugins::module_presets(&module_id).iter())
            .map(|p| PresetEntryDto {
                key: p.key.clone(),
                name: (&p.name).into(),
                is_factory: true,
            })
            .collect();
        let user = store.list(&module_id).map_err(preset_ipc_error)?;
        out.extend(user.into_iter().map(user_entry));
        Ok(out)
    })
    .await
    .map_err(join_blocking_err)?
}

/// Saves slot `slot`'s current committed state as a new user preset named `name`. The noise
/// print (if the slot has one) is included only when `include_noise_print` is set (SPEC-012
/// scope: "include noise print" checkbox, off by default). `overwrite` replaces an existing
/// preset of the same name instead of failing.
#[tauri::command]
pub async fn module_preset_save(
    engine: State<'_, AudioEngine>,
    presets: State<'_, PresetStores>,
    slot: usize,
    name: String,
    include_noise_print: bool,
    overwrite: bool,
) -> Result<PresetEntryDto, IpcError> {
    let handle = engine.handle().clone();
    let store = presets.modules.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let module_id = handle.rack_slot_module_id(slot).ok_or_else(|| {
            IpcError::new(IpcErrorCode::InvalidArgument, "error.rack_rejected").with_param(
                "message",
                format!("slot {slot} has no module to save a preset from"),
            )
        })?;
        let mut state = handle.rack_slot_state(slot).map_err(rack_ipc_error)?;
        if !include_noise_print {
            state.blob = None;
        }
        save_module_preset(&store, &module_id, &name, &state, overwrite)
    })
    .await
    .map_err(join_blocking_err)?
}

fn save_module_preset(
    store: &ModulePresetStore,
    module_id: &str,
    name: &str,
    state: &ModuleState,
    overwrite: bool,
) -> Result<PresetEntryDto, IpcError> {
    match store.save(module_id, name, state) {
        Ok(saved) => Ok(user_entry(saved)),
        Err(PresetError::AlreadyExists(sanitized)) if overwrite => {
            store
                .delete(module_id, &sanitized)
                .map_err(preset_ipc_error)?;
            let saved = store
                .save(module_id, name, state)
                .map_err(preset_ipc_error)?;
            Ok(user_entry(saved))
        }
        Err(e) => Err(preset_ipc_error(e)),
    }
}

/// Writes user preset `name` (module `module_id`) to `path` (H-22, SPEC-012 §2.7 "Export…"): any
/// filesystem path, chosen by the caller's native save dialog — a portable file another
/// PowerVoice install (or this one, later) can import back with [`import_module_preset_bytes`].
fn export_module_preset(
    store: &ModulePresetStore,
    module_id: &str,
    name: &str,
    dest: &Path,
) -> Result<(), IpcError> {
    let state = store.load(module_id, name).map_err(preset_ipc_error)?;
    let exported = ExportedPreset::module(module_id, name, state);
    export_to_file(&exported, dest).map_err(preset_ipc_error)
}

/// Parses an exported preset file's `bytes` and saves it as a user module preset (H-22 "Import…"):
/// the `module_id`/`name` the file itself claims, sanitized the normal way by
/// [`save_module_preset`] — never trusted verbatim. Rejects a rack-preset file (wrong `kind`)
/// with the same `error.preset_rejected` shape as any other malformed input.
fn import_module_preset_bytes(
    store: &ModulePresetStore,
    bytes: &[u8],
    overwrite: bool,
) -> Result<ModulePresetImportedDto, IpcError> {
    match import_from_file(bytes).map_err(preset_ipc_error)? {
        ExportedPreset::Module {
            module_id,
            name,
            state,
            ..
        } => {
            let entry = save_module_preset(store, &module_id, &name, &state, overwrite)?;
            Ok(ModulePresetImportedDto { module_id, entry })
        }
        ExportedPreset::Rack { .. } => Err(wrong_kind_error()),
    }
}

fn wrong_kind_error() -> IpcError {
    IpcError::new(IpcErrorCode::InvalidArgument, "error.preset_rejected").with_param(
        "message",
        "not a preset file of the expected kind".to_owned(),
    )
}

/// Loads a module preset into slot `slot` (SPEC-012 §2.7): a state with a blob replaces the
/// instance behind a 15 ms crossfade; a parameter-only state is smoothed, with no restart.
#[tauri::command]
pub async fn module_preset_load(
    engine: State<'_, AudioEngine>,
    documents: State<'_, DocumentService>,
    presets: State<'_, PresetStores>,
    slot: usize,
    module_id: String,
    preset: PresetRefDto,
) -> Result<RackStateDto, IpcError> {
    let handle = engine.handle().clone();
    let store = presets.modules.clone();
    let state = tauri::async_runtime::spawn_blocking(move || {
        resolve_module_preset(&handle, &store, &module_id, preset)
    })
    .await
    .map_err(join_blocking_err)??;
    apply(
        &engine,
        &documents,
        RackCommand::ApplyModulePreset { index: slot, state },
    )
    .await
}

fn resolve_module_preset(
    handle: &vox_engine::EngineHandle,
    store: &vox_presets::ModulePresetStore,
    module_id: &str,
    preset: PresetRefDto,
) -> Result<ModuleState, IpcError> {
    match preset {
        PresetRefDto::Factory { key } => handle
            .rack_module_presets(module_id)
            .into_iter()
            .chain(crate::plugins::module_presets(module_id))
            .find(|p| p.key == key)
            .map(|p| p.state)
            .ok_or_else(|| no_such_preset(&key)),
        PresetRefDto::User { name } => store.load(module_id, &name).map_err(preset_ipc_error),
    }
}

fn no_such_preset(key: &str) -> IpcError {
    IpcError::new(IpcErrorCode::NotFound, "error.preset_rejected")
        .with_param("message", format!("no preset `{key}`"))
}

/// Resets every writable parameter of slot `slot` to its schema default (T-406): smoothed, no
/// restart; a committed blob (e.g. a noise print) is kept.
#[tauri::command]
pub async fn module_reset_default(
    engine: State<'_, AudioEngine>,
    documents: State<'_, DocumentService>,
    slot: usize,
) -> Result<RackStateDto, IpcError> {
    apply(
        &engine,
        &documents,
        RackCommand::ResetToDefault { index: slot },
    )
    .await
}

/// Renames a user module preset.
#[tauri::command]
pub async fn module_preset_rename(
    presets: State<'_, PresetStores>,
    module_id: String,
    old_name: String,
    new_name: String,
) -> Result<PresetEntryDto, IpcError> {
    let store = presets.modules.clone();
    tauri::async_runtime::spawn_blocking(move || {
        store
            .rename(&module_id, &old_name, &new_name)
            .map(user_entry)
            .map_err(preset_ipc_error)
    })
    .await
    .map_err(join_blocking_err)?
}

/// Deletes a user module preset.
#[tauri::command]
pub async fn module_preset_delete(
    presets: State<'_, PresetStores>,
    module_id: String,
    name: String,
) -> Result<(), IpcError> {
    let store = presets.modules.clone();
    tauri::async_runtime::spawn_blocking(move || {
        store.delete(&module_id, &name).map_err(preset_ipc_error)
    })
    .await
    .map_err(join_blocking_err)?
}

/// Exports user module preset `name` (of `module_id`) to `path` (H-22, Manage Presets… "Export…";
/// `path` came from the caller's native save dialog).
#[tauri::command]
pub async fn module_preset_export(
    presets: State<'_, PresetStores>,
    module_id: String,
    name: String,
    path: String,
) -> Result<(), IpcError> {
    let store = presets.modules.clone();
    tauri::async_runtime::spawn_blocking(move || {
        export_module_preset(&store, &module_id, &name, Path::new(&path))
    })
    .await
    .map_err(join_blocking_err)?
}

/// Imports a module preset file at `path` (H-22, Manage Presets… "Import…"; `path` came from the
/// caller's native open dialog). `overwrite` replaces an existing preset of the same name, exactly
/// like `module_preset_save`.
#[tauri::command]
pub async fn module_preset_import(
    presets: State<'_, PresetStores>,
    path: String,
    overwrite: bool,
) -> Result<ModulePresetImportedDto, IpcError> {
    let store = presets.modules.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let bytes = std::fs::read(&path).map_err(IpcError::from)?;
        import_module_preset_bytes(&store, &bytes, overwrite)
    })
    .await
    .map_err(join_blocking_err)?
}

// --- Rack presets ------------------------------------------------------------------------------

/// Factory rack presets ("Podcast voice", "Audiobook (ACX)", "Gentle cleanup", ...) first, then
/// user-saved ones (alphabetical) — Effects → Rack Presets (SPEC-012 "the rack-preset menu").
#[tauri::command]
pub async fn rack_presets_list(
    presets: State<'_, PresetStores>,
) -> Result<Vec<PresetEntryDto>, IpcError> {
    let store = presets.racks.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let mut out: Vec<PresetEntryDto> = vox_presets::factory_rack_presets()
            .into_iter()
            .map(|p| PresetEntryDto {
                key: p.key,
                name: (&p.name).into(),
                is_factory: true,
            })
            .collect();
        let user = store.list().map_err(preset_ipc_error)?;
        out.extend(user.into_iter().map(user_entry));
        Ok(out)
    })
    .await
    .map_err(join_blocking_err)?
}

fn save_rack_preset(
    store: &RackPresetStore,
    name: &str,
    model: &RackModel,
    overwrite: bool,
) -> Result<PresetEntryDto, IpcError> {
    match store.save(name, model) {
        Ok(saved) => Ok(user_entry(saved)),
        Err(PresetError::AlreadyExists(sanitized)) if overwrite => {
            store.delete(&sanitized).map_err(preset_ipc_error)?;
            store
                .save(name, model)
                .map(user_entry)
                .map_err(preset_ipc_error)
        }
        Err(e) => Err(preset_ipc_error(e)),
    }
}

/// Writes user rack preset `name` to `path` (H-22 "Export…"; any filesystem path, chosen by the
/// caller's native save dialog).
fn export_rack_preset(store: &RackPresetStore, name: &str, dest: &Path) -> Result<(), IpcError> {
    let model = store.load(name).map_err(preset_ipc_error)?;
    let exported = ExportedPreset::rack(name, model);
    export_to_file(&exported, dest).map_err(preset_ipc_error)
}

/// Parses an exported preset file's `bytes` and saves it as a user rack preset (H-22 "Import…"):
/// the `name` inside is sanitized the normal way by [`save_rack_preset`] — never trusted
/// verbatim. An unknown module id in an imported rack round-trips exactly like any other stored
/// rack preset (`RackModel`/`SlotModel` already handle that generically) — it becomes a
/// placeholder slot with a notice only once the preset is actually loaded, not on import.
fn import_rack_preset_bytes(
    store: &RackPresetStore,
    bytes: &[u8],
    overwrite: bool,
) -> Result<PresetEntryDto, IpcError> {
    match import_from_file(bytes).map_err(preset_ipc_error)? {
        ExportedPreset::Rack { name, rack, .. } => save_rack_preset(store, &name, &rack, overwrite),
        ExportedPreset::Module { .. } => Err(wrong_kind_error()),
    }
}

/// Saves the live rack as a new user rack preset named `name` (SPEC-012 "save the whole chain").
/// `overwrite` replaces an existing preset of the same name instead of failing.
#[tauri::command]
pub async fn rack_preset_save(
    engine: State<'_, AudioEngine>,
    presets: State<'_, PresetStores>,
    name: String,
    overwrite: bool,
) -> Result<PresetEntryDto, IpcError> {
    let handle = engine.handle().clone();
    let store = presets.racks.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let model = handle
            .rack_model()
            .ok_or_else(|| IpcError::new(IpcErrorCode::Internal, "error.rack_unavailable"))?;
        save_rack_preset(&store, &name, &model, overwrite)
    })
    .await
    .map_err(join_blocking_err)?
}

/// Loads a rack preset, replacing the live rack (SPEC-012 §2.1's `rack_load_model` path): an
/// unknown module id becomes a placeholder slot with a notice, exactly like opening a document
/// whose sidecar names an uninstalled module — never a load failure.
#[tauri::command]
pub async fn rack_preset_load(
    engine: State<'_, AudioEngine>,
    documents: State<'_, DocumentService>,
    presets: State<'_, PresetStores>,
    preset: PresetRefDto,
) -> Result<RackStateDto, IpcError> {
    if documents.is_bake_running() {
        return Err(crate::ipc::rack_commands::bake_busy());
    }
    let store = presets.racks.clone();
    let model = tauri::async_runtime::spawn_blocking(move || match preset {
        PresetRefDto::Factory { key } => vox_presets::factory_rack_presets()
            .into_iter()
            .find(|p| p.key == key)
            .map(|p| p.model)
            .ok_or_else(|| no_such_preset(&key)),
        PresetRefDto::User { name } => store.load(&name).map_err(preset_ipc_error),
    })
    .await
    .map_err(join_blocking_err)??;

    let handle = engine.handle().clone();
    let result = tauri::async_runtime::spawn_blocking(move || handle.rack_load_model(model))
        .await
        .map_err(join_blocking_err)?;
    let snapshot =
        result.ok_or_else(|| IpcError::new(IpcErrorCode::Internal, "error.rack_unavailable"))?;
    Ok(RackStateDto::from(&snapshot.map_err(rack_ipc_error)?))
}

/// Renames a user rack preset.
#[tauri::command]
pub async fn rack_preset_rename(
    presets: State<'_, PresetStores>,
    old_name: String,
    new_name: String,
) -> Result<PresetEntryDto, IpcError> {
    let store = presets.racks.clone();
    tauri::async_runtime::spawn_blocking(move || {
        store
            .rename(&old_name, &new_name)
            .map(user_entry)
            .map_err(preset_ipc_error)
    })
    .await
    .map_err(join_blocking_err)?
}

/// Deletes a user rack preset.
#[tauri::command]
pub async fn rack_preset_delete(
    presets: State<'_, PresetStores>,
    name: String,
) -> Result<(), IpcError> {
    let store = presets.racks.clone();
    tauri::async_runtime::spawn_blocking(move || store.delete(&name).map_err(preset_ipc_error))
        .await
        .map_err(join_blocking_err)?
}

/// Exports user rack preset `name` to `path` (H-22, Manage Presets… "Export…"; `path` came from
/// the caller's native save dialog).
#[tauri::command]
pub async fn rack_preset_export(
    presets: State<'_, PresetStores>,
    name: String,
    path: String,
) -> Result<(), IpcError> {
    let store = presets.racks.clone();
    tauri::async_runtime::spawn_blocking(move || {
        export_rack_preset(&store, &name, Path::new(&path))
    })
    .await
    .map_err(join_blocking_err)?
}

/// Imports a rack preset file at `path` (H-22, Manage Presets… "Import…"; `path` came from the
/// caller's native open dialog). `overwrite` replaces an existing preset of the same name, exactly
/// like `rack_preset_save`.
#[tauri::command]
pub async fn rack_preset_import(
    presets: State<'_, PresetStores>,
    path: String,
    overwrite: bool,
) -> Result<PresetEntryDto, IpcError> {
    let store = presets.racks.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let bytes = std::fs::read(&path).map_err(IpcError::from)?;
        import_rack_preset_bytes(&store, &bytes, overwrite)
    })
    .await
    .map_err(join_blocking_err)?
}

#[cfg(test)]
#[allow(clippy::float_cmp)] // exact values
mod tests {
    use std::collections::BTreeMap;

    use vox_module_api::{ModuleRef, Version};
    use vox_rack::SlotModel;

    use super::*;

    fn tmp_dir(tag: &str) -> std::path::PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "powervoice-preset-commands-test-{tag}-{}-{nanos}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn state(gain_db: f64) -> ModuleState {
        ModuleState {
            format_version: 1,
            params: BTreeMap::from([("gain_db".to_owned(), gain_db)]),
            blob: None,
        }
    }

    #[test]
    fn save_module_preset_without_overwrite_fails_on_a_duplicate_name() {
        let store = ModulePresetStore::new(tmp_dir("no-overwrite"));
        save_module_preset(&store, "org.powervoice.gain", "Mine", &state(1.0), false).unwrap();
        let err = save_module_preset(&store, "org.powervoice.gain", "Mine", &state(2.0), false)
            .unwrap_err();
        assert_eq!(err.code, IpcErrorCode::InvalidArgument);
        // The original is untouched.
        assert_eq!(
            store.load("org.powervoice.gain", "Mine").unwrap().params["gain_db"],
            1.0
        );
    }

    #[test]
    fn save_module_preset_with_overwrite_replaces_the_existing_preset() {
        let store = ModulePresetStore::new(tmp_dir("overwrite"));
        save_module_preset(&store, "org.powervoice.gain", "Mine", &state(1.0), false).unwrap();
        let entry =
            save_module_preset(&store, "org.powervoice.gain", "Mine", &state(2.0), true).unwrap();
        assert_eq!(entry.key, "Mine");
        assert_eq!(
            store.load("org.powervoice.gain", "Mine").unwrap().params["gain_db"],
            2.0
        );
    }

    #[test]
    fn preset_ipc_error_maps_not_found_and_invalid_name() {
        assert_eq!(
            preset_ipc_error(vox_presets::PresetError::NotFound("x".into())).code,
            IpcErrorCode::NotFound
        );
        assert_eq!(
            preset_ipc_error(vox_presets::PresetError::InvalidName("..".into())).code,
            IpcErrorCode::InvalidArgument
        );
    }

    /// H-22: `AlreadyExists` gets its own key (plus the conflicting name as a structured param)
    /// so the UI can offer "Replace preset ‹name›?" instead of just reporting a generic failure.
    #[test]
    fn preset_ipc_error_gives_already_exists_its_own_key_and_carries_the_name() {
        let err = preset_ipc_error(vox_presets::PresetError::AlreadyExists("Mine".into()));
        assert_eq!(err.code, IpcErrorCode::InvalidArgument);
        assert_eq!(err.key, "error.preset_already_exists");
        assert_eq!(err.params.get("name").map(String::as_str), Some("Mine"));
    }

    fn gain_slot(db: f64) -> SlotModel {
        SlotModel::new(
            &ModuleRef {
                id: "org.powervoice.gain".into(),
                version: Version::new(1, 0, 0),
            },
            false,
            &state(db),
        )
    }

    #[test]
    fn module_preset_export_then_import_round_trips_into_a_fresh_store() {
        let store = ModulePresetStore::new(tmp_dir("export-module"));
        save_module_preset(&store, "org.powervoice.gain", "Mine", &state(2.0), false).unwrap();
        let dest = tmp_dir("export-module-file").join("Mine.json");
        export_module_preset(&store, "org.powervoice.gain", "Mine", &dest).unwrap();

        let fresh = ModulePresetStore::new(tmp_dir("import-module"));
        let bytes = std::fs::read(&dest).unwrap();
        let imported = import_module_preset_bytes(&fresh, &bytes, false).unwrap();
        assert_eq!(imported.module_id, "org.powervoice.gain");
        assert_eq!(imported.entry.key, "Mine");
        assert_eq!(
            fresh.load("org.powervoice.gain", "Mine").unwrap().params["gain_db"],
            2.0
        );
    }

    #[test]
    fn importing_a_module_preset_over_an_existing_name_needs_overwrite() {
        let dest = tmp_dir("import-conflict-file").join("Mine.json");
        export_to_file(
            &ExportedPreset::module("org.powervoice.gain", "Mine", state(9.0)),
            &dest,
        )
        .unwrap();
        let store = ModulePresetStore::new(tmp_dir("import-conflict"));
        save_module_preset(&store, "org.powervoice.gain", "Mine", &state(1.0), false).unwrap();

        let bytes = std::fs::read(&dest).unwrap();
        let err = import_module_preset_bytes(&store, &bytes, false).unwrap_err();
        assert_eq!(err.key, "error.preset_already_exists");
        assert_eq!(
            store.load("org.powervoice.gain", "Mine").unwrap().params["gain_db"],
            1.0,
            "the original is untouched without overwrite"
        );

        let imported = import_module_preset_bytes(&store, &bytes, true).unwrap();
        assert_eq!(imported.entry.key, "Mine");
        assert_eq!(
            store.load("org.powervoice.gain", "Mine").unwrap().params["gain_db"],
            9.0
        );
    }

    #[test]
    fn importing_malformed_bytes_is_rejected_not_a_panic() {
        let store = ModulePresetStore::new(tmp_dir("import-malformed"));
        let err = import_module_preset_bytes(&store, b"{ not json", false).unwrap_err();
        assert_eq!(err.key, "error.preset_rejected");
    }

    #[test]
    fn importing_a_rack_preset_file_as_a_module_preset_is_rejected() {
        let store = ModulePresetStore::new(tmp_dir("import-wrong-kind"));
        let dest = tmp_dir("import-wrong-kind-file").join("Rack.json");
        export_to_file(
            &ExportedPreset::rack("Some rack", RackModel { slots: vec![] }),
            &dest,
        )
        .unwrap();
        let bytes = std::fs::read(&dest).unwrap();
        let err = import_module_preset_bytes(&store, &bytes, false).unwrap_err();
        assert_eq!(err.code, IpcErrorCode::InvalidArgument);
    }

    #[test]
    fn importing_a_module_preset_file_as_a_rack_preset_is_rejected() {
        let store = RackPresetStore::new(tmp_dir("import-wrong-kind-rack"));
        let dest = tmp_dir("import-wrong-kind-rack-file").join("Module.json");
        export_to_file(
            &ExportedPreset::module("org.powervoice.gain", "Mine", state(0.0)),
            &dest,
        )
        .unwrap();
        let bytes = std::fs::read(&dest).unwrap();
        let err = import_rack_preset_bytes(&store, &bytes, false).unwrap_err();
        assert_eq!(err.code, IpcErrorCode::InvalidArgument);
    }

    #[test]
    fn importing_a_name_that_looks_like_path_traversal_never_escapes_the_module_store() {
        let root = tmp_dir("import-traversal");
        let store = ModulePresetStore::new(root.clone());
        let dest = tmp_dir("import-traversal-file").join("evil.json");
        export_to_file(
            &ExportedPreset::module("org.powervoice.gain", "evil/../../etc", state(0.0)),
            &dest,
        )
        .unwrap();
        let bytes = std::fs::read(&dest).unwrap();
        let imported = import_module_preset_bytes(&store, &bytes, false).unwrap();
        // Whatever `sanitize_preset_name` turned the name into, it's still a single path
        // component inside `org.powervoice.gain`'s own directory.
        assert!(!imported.entry.key.contains('/') && !imported.entry.key.contains('\\'));
        assert!(
            !root
                .parent()
                .unwrap()
                .join(format!("{}.json", imported.entry.key))
                .exists()
        );
    }

    #[test]
    fn rack_preset_export_then_import_round_trips_an_unknown_module_id() {
        // ADR-005 §2: importing never special-cases an unknown module id — the preset round-trips
        // verbatim (`RackModel`/`SlotModel` already handle that) and only becomes a placeholder
        // slot with a notice once it's actually loaded into the live rack, not on import.
        let store = RackPresetStore::new(tmp_dir("export-rack"));
        let rack = RackModel {
            slots: vec![
                gain_slot(-3.0),
                SlotModel::new(
                    &ModuleRef {
                        id: "org.example.future-plugin".into(),
                        version: Version::new(2, 0, 0),
                    },
                    false,
                    &ModuleState::new(1),
                ),
            ],
        };
        save_rack_preset(&store, "Mine", &rack, false).unwrap();
        let dest = tmp_dir("export-rack-file").join("Mine.json");
        export_rack_preset(&store, "Mine", &dest).unwrap();

        let fresh = RackPresetStore::new(tmp_dir("import-rack"));
        let bytes = std::fs::read(&dest).unwrap();
        let entry = import_rack_preset_bytes(&fresh, &bytes, false).unwrap();
        assert_eq!(entry.key, "Mine");
        assert_eq!(fresh.load("Mine").unwrap(), rack);
    }

    #[test]
    fn importing_a_rack_preset_over_an_existing_name_needs_overwrite() {
        let dest = tmp_dir("import-rack-conflict-file").join("Mine.json");
        export_to_file(
            &ExportedPreset::rack(
                "Mine",
                RackModel {
                    slots: vec![gain_slot(6.0)],
                },
            ),
            &dest,
        )
        .unwrap();
        let store = RackPresetStore::new(tmp_dir("import-rack-conflict"));
        let original = RackModel {
            slots: vec![gain_slot(0.0)],
        };
        save_rack_preset(&store, "Mine", &original, false).unwrap();

        let bytes = std::fs::read(&dest).unwrap();
        let err = import_rack_preset_bytes(&store, &bytes, false).unwrap_err();
        assert_eq!(err.key, "error.preset_already_exists");
        assert_eq!(store.load("Mine").unwrap(), original);

        import_rack_preset_bytes(&store, &bytes, true).unwrap();
        assert_ne!(store.load("Mine").unwrap(), original);
    }
}

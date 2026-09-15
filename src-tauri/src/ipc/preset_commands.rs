//! Module & rack preset commands (T-406, SPEC-012 §2.7, ADR-005 §2/§10/§12). Thin: storage,
//! validation and factory-preset content live in `vox_presets`; applying a loaded preset to the
//! live rack forwards a `vox_engine::RackCommand` exactly like every other rack command
//! (`rack_commands::apply`).

use tauri::State;
use vox_engine::RackCommand;
use vox_rack::ModuleState;

use crate::audio::AudioEngine;
use crate::document::DocumentService;
use crate::ipc::error::{IpcError, IpcErrorCode};
use crate::ipc::preset_dto::{PresetEntryDto, PresetRefDto, preset_ipc_error};
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

/// Factory presets (`ModuleFactory::presets`, ADR-005 §2) first, then user-saved ones
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
    store: &vox_presets::ModulePresetStore,
    module_id: &str,
    name: &str,
    state: &ModuleState,
    overwrite: bool,
) -> Result<PresetEntryDto, IpcError> {
    match store.save(module_id, name, state) {
        Ok(saved) => Ok(user_entry(saved)),
        Err(vox_presets::PresetError::AlreadyExists(sanitized)) if overwrite => {
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
        match store.save(&name, &model) {
            Ok(saved) => Ok(user_entry(saved)),
            Err(vox_presets::PresetError::AlreadyExists(sanitized)) if overwrite => {
                store.delete(&sanitized).map_err(preset_ipc_error)?;
                store
                    .save(&name, &model)
                    .map(user_entry)
                    .map_err(preset_ipc_error)
            }
            Err(e) => Err(preset_ipc_error(e)),
        }
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

#[cfg(test)]
#[allow(clippy::float_cmp)] // exact values
mod tests {
    use std::collections::BTreeMap;

    use vox_presets::ModulePresetStore;

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
}

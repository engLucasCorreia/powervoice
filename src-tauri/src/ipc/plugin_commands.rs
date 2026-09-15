//! Plugin manager commands (T-804 item 6; thin — the business logic lives in
//! `vox_plugin_host::catalog`. The plugin manager UI itself is T-809).

use tauri::State;

use crate::ipc::error::IpcError;
use crate::ipc::plugin_dto::{PluginEntryDto, blocklisted_entry, clap_entry};
use crate::settings::SettingsStore;

/// Every known plugin (T-804 item 6): registered ones (ok / disabled / flagged) plus blocklisted
/// files. Sorted by id, blocklisted entries (empty id) last.
#[tauri::command]
pub async fn plugins_list(
    settings: State<'_, SettingsStore>,
) -> Result<Vec<PluginEntryDto>, IpcError> {
    let disabled = settings.get().plugins.disabled;
    let health = crate::plugins::health();
    let mut list: Vec<PluginEntryDto> = crate::plugins::registry_specs()
        .iter()
        .map(|spec| {
            let flag = health
                .as_ref()
                .map(|h| h.get(&spec.descriptor.id))
                .unwrap_or_default();
            clap_entry(spec, &disabled, flag)
        })
        .collect();
    list.sort_by(|a, b| a.id.cmp(&b.id));
    for (path, info) in crate::plugins::blocklist_snapshot() {
        list.push(blocklisted_entry(&path, &info));
    }
    Ok(list)
}

/// Rescans for plugins (T-804 item 6). `full`: ignores the cache — every file is scanned again,
/// even an unchanged one (blocklisted files are still skipped; that's `plugins_unblock`'s job).
/// Runs synchronously off the calling thread; hot-adds anything new into every live registry.
#[tauri::command]
pub async fn plugins_rescan(full: bool) -> Result<u32, IpcError> {
    let summary = tauri::async_runtime::spawn_blocking(move || crate::plugins::rescan(full))
        .await
        .map_err(|e| IpcError::internal(e.to_string()))?;
    Ok(summary.effects as u32)
}

/// Shows/hides `module_id` in Add Module (T-804 item 5). It still loads for a document that
/// already uses it — this only changes what `rack_list_modules` returns.
#[tauri::command]
pub async fn plugins_set_enabled(
    settings: State<'_, SettingsStore>,
    module_id: String,
    enabled: bool,
) -> Result<(), IpcError> {
    let mut next = settings.get();
    next.plugins.disabled.retain(|id| id != &module_id);
    if !enabled {
        next.plugins.disabled.push(module_id);
    }
    settings.set(next)?;
    Ok(())
}

/// Blocks a plugin file manually (T-804 item 4).
#[tauri::command]
pub async fn plugins_block(path: String) -> Result<(), IpcError> {
    crate::plugins::block(std::path::Path::new(&path));
    Ok(())
}

/// Unblocks a plugin file (T-804 item 4); `true` if it was blocked.
#[tauri::command]
pub async fn plugins_unblock(path: String) -> Result<bool, IpcError> {
    Ok(crate::plugins::unblock(std::path::Path::new(&path)))
}

/// Adds a custom scan folder (T-804 item 5, ADR-008 §6) and rescans so it's picked up right
/// away.
#[tauri::command]
pub async fn plugins_add_folder(
    settings: State<'_, SettingsStore>,
    path: String,
) -> Result<(), IpcError> {
    let mut next = settings.get();
    if !next.plugins.custom_folders.iter().any(|p| p == &path) {
        next.plugins.custom_folders.push(path);
    }
    let saved = settings.set(next)?;
    crate::plugins::configure(&saved.plugins.custom_folders);
    tauri::async_runtime::spawn_blocking(|| crate::plugins::rescan(false));
    Ok(())
}

/// Removes a custom scan folder (T-804 item 5). Plugins already registered from it stay
/// registered until the app restarts (matches the "a plugin removed from disk isn't torn out
/// from underneath the live rack" policy elsewhere — SPEC-012).
#[tauri::command]
pub async fn plugins_remove_folder(
    settings: State<'_, SettingsStore>,
    path: String,
) -> Result<(), IpcError> {
    let mut next = settings.get();
    next.plugins.custom_folders.retain(|p| p != &path);
    let saved = settings.set(next)?;
    crate::plugins::configure(&saved.plugins.custom_folders);
    Ok(())
}

//! Plugin manager commands (T-804 item 6, T-809; thin — the business logic lives in
//! `vox_plugin_host::catalog`/`install`. The plugin manager UI is `ui/src/lib/plugins/`).

use tauri::{AppHandle, State};

use crate::ipc::error::{IpcError, IpcErrorCode};
use crate::ipc::events::{emit_plugin_scan_progress, emit_plugin_scan_summary};
use crate::ipc::plugin_dto::{
    PluginEntryDto, PluginFoldersDto, PluginInstallResultDto, plugin_list,
};
use crate::settings::SettingsStore;

/// Rescans, reporting every step through `plugin_scan_progress` and the summary at the end
/// (T-809: the plugin manager's progress bar follows every rescan, not only the start-up one).
fn rescan_emitting<R: tauri::Runtime>(app: &AppHandle<R>, full: bool) -> u32 {
    let progress_app = app.clone();
    let summary = crate::plugins::rescan_with(full, move |done, total, path| {
        if let Err(e) = emit_plugin_scan_progress(&progress_app, done, total, path) {
            tracing::warn!(error = %e, "plugin_scan_progress emit failed");
        }
    });
    let effects = summary.effects as u32;
    if let Err(e) = emit_plugin_scan_summary(app, summary) {
        tracing::warn!(error = %e, "plugin_scan_progress summary emit failed");
    }
    effects
}

/// Every known plugin (T-804 item 6): registered ones (ok / disabled / flagged, or blocklisted
/// when their file has been blocked since) plus the other blocklisted files.
#[tauri::command]
pub async fn plugins_list(
    settings: State<'_, SettingsStore>,
) -> Result<Vec<PluginEntryDto>, IpcError> {
    let disabled = settings.get().plugins.disabled;
    let health = crate::plugins::health();
    Ok(plugin_list(
        &crate::plugins::registry_specs(),
        crate::plugins::details,
        &disabled,
        |id| health.as_ref().map(|h| h.get(id)).unwrap_or_default(),
        &crate::plugins::blocklist_snapshot(),
    ))
}

/// Rescans for plugins (T-804 item 6). `full`: ignores the cache — every file is scanned again,
/// even an unchanged one (blocklisted files are still skipped; that's `plugins_unblock`'s job).
/// Emits `plugin_scan_progress` as it goes; hot-adds anything new into every live registry.
#[tauri::command]
pub async fn plugins_rescan<R: tauri::Runtime>(
    app: AppHandle<R>,
    full: bool,
) -> Result<u32, IpcError> {
    tauri::async_runtime::spawn_blocking(move || rescan_emitting(&app, full))
        .await
        .map_err(|e| IpcError::internal(e.to_string()))
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

/// Unblocks a plugin file (T-804 item 4); `true` if it was blocked. The UI rescans afterwards
/// so the file is picked up again.
#[tauri::command]
pub async fn plugins_unblock(path: String) -> Result<bool, IpcError> {
    Ok(crate::plugins::unblock(std::path::Path::new(&path)))
}

/// Clears a flagged plugin's runtime crash count (T-809, "Clear crash warning").
#[tauri::command]
pub async fn plugins_clear_flag(module_id: String) -> Result<(), IpcError> {
    crate::plugins::clear_flag(&module_id);
    Ok(())
}

/// Adds a custom scan folder (T-804 item 5, ADR-008 §6) and rescans in the background (with
/// `plugin_scan_progress`) so it's picked up right away.
#[tauri::command]
pub async fn plugins_add_folder<R: tauri::Runtime>(
    app: AppHandle<R>,
    settings: State<'_, SettingsStore>,
    path: String,
) -> Result<(), IpcError> {
    let mut next = settings.get();
    if !next.plugins.custom_folders.iter().any(|p| p == &path) {
        next.plugins.custom_folders.push(path);
    }
    let saved = settings.set(next)?;
    crate::plugins::configure(&saved.plugins.custom_folders);
    tauri::async_runtime::spawn_blocking(move || rescan_emitting(&app, false));
    Ok(())
}

/// Removes a custom scan folder (T-804 item 5) and rescans in the background (T-809 item 2), so
/// its plugins leave the plugin manager's list. Plugins already registered from it stay
/// registered until the app restarts (matches the "a plugin removed from disk isn't torn out
/// from underneath the live rack" policy elsewhere — SPEC-012).
#[tauri::command]
pub async fn plugins_remove_folder<R: tauri::Runtime>(
    app: AppHandle<R>,
    settings: State<'_, SettingsStore>,
    path: String,
) -> Result<(), IpcError> {
    let mut next = settings.get();
    next.plugins.custom_folders.retain(|p| p != &path);
    let saved = settings.set(next)?;
    crate::plugins::configure(&saved.plugins.custom_folders);
    tauri::async_runtime::spawn_blocking(move || rescan_emitting(&app, false));
    Ok(())
}

/// The install folder, the standard folders and the custom ones (T-809).
#[tauri::command]
pub async fn plugins_folders(
    settings: State<'_, SettingsStore>,
) -> Result<PluginFoldersDto, IpcError> {
    let text = |p: std::path::PathBuf| p.to_string_lossy().into_owned();
    Ok(PluginFoldersDto {
        install: crate::plugins::install_dir().map(text),
        standard: crate::plugins::standard_folders()
            .into_iter()
            .map(text)
            .collect(),
        custom: settings.get().plugins.custom_folders,
    })
}

/// "Install module…" (T-809): copies the picked `.clap` into the per-user plugin folder, scans
/// only that file (up to the 30 s scan timeout, off the async runtime) and registers it. A name
/// collision comes back as `collision` without changing anything, until `replace` is set.
#[tauri::command]
pub async fn plugins_install(
    path: String,
    replace: bool,
) -> Result<PluginInstallResultDto, IpcError> {
    let result = tauri::async_runtime::spawn_blocking(move || {
        crate::plugins::install(std::path::Path::new(&path), replace)
    })
    .await
    .map_err(|e| IpcError::internal(e.to_string()))?;
    Ok(result.into())
}

/// Shows a plugin file in the system file manager (T-809).
#[tauri::command]
pub async fn plugins_reveal(path: String) -> Result<(), IpcError> {
    crate::plugins::reveal(std::path::Path::new(&path)).map_err(|e| {
        IpcError::new(IpcErrorCode::Io, "error.plugins.reveal").with_param("message", e.to_string())
    })
}

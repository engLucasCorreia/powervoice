//! Recent files commands (T-306, SPEC-018 §2.12). Thin: state lives in
//! [`crate::settings::SettingsStore`]; this module only maps to/from the DTO and fires the
//! `recent_files_changed` event.

use tauri::{AppHandle, Emitter, Runtime, State};

use crate::ipc::error::IpcError;
use crate::ipc::events::EventName;
use crate::ipc::recent_files_dto::RecentFileDto;
use crate::settings::{self, SettingsStore};

fn to_dtos(entries: &[settings::RecentFileEntry]) -> Vec<RecentFileDto> {
    entries.iter().map(RecentFileDto::from).collect()
}

fn emit_recent_files_changed<R: Runtime>(
    app: &AppHandle<R>,
    entries: &[settings::RecentFileEntry],
) {
    if let Err(error) = app.emit(EventName::recent_files_changed.as_str(), to_dtos(entries)) {
        tracing::warn!(%error, "emitting recent_files_changed failed");
    }
}

/// The current list, most-recent-first (SPEC-018 §2.12).
#[tauri::command]
pub async fn recent_files_get(
    settings: State<'_, SettingsStore>,
) -> Result<Vec<RecentFileDto>, IpcError> {
    Ok(to_dtos(&settings.get().recent_files))
}

/// Removes one entry (the File menu's "Remove" action on a missing file, or "Remove from List").
#[tauri::command]
pub async fn recent_files_remove<R: Runtime>(
    app: AppHandle<R>,
    settings: State<'_, SettingsStore>,
    path: String,
) -> Result<Vec<RecentFileDto>, IpcError> {
    let mut current = settings.get();
    settings::remove_recent_file(&mut current.recent_files, &path);
    let saved = settings.set(current)?;
    emit_recent_files_changed(&app, &saved.recent_files);
    Ok(to_dtos(&saved.recent_files))
}

/// Empties the list (SPEC-018 §2.12: "deletes no user data" — no confirmation needed).
#[tauri::command]
pub async fn recent_files_clear<R: Runtime>(
    app: AppHandle<R>,
    settings: State<'_, SettingsStore>,
) -> Result<(), IpcError> {
    let mut current = settings.get();
    current.recent_files.clear();
    let saved = settings.set(current)?;
    emit_recent_files_changed(&app, &saved.recent_files);
    Ok(())
}

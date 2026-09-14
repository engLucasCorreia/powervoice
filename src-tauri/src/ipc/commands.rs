use tauri::State;

use crate::ipc::dto::AppInfo;
use crate::ipc::error::IpcError;
use crate::settings::{Settings, SettingsStore};

/// Returns basic app identity info (name + version). Tauri command handlers are always `async`
/// so sync commands still run off the main thread (CLAUDE.md / ADR-003).
#[tauri::command]
pub async fn app_info() -> Result<AppInfo, IpcError> {
    Ok(AppInfo {
        name: "PowerVoice".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
    })
}

/// Returns the current settings (T-104). Served from the in-memory `SettingsStore` cache — this
/// command never touches disk.
#[tauri::command]
pub async fn settings_get(store: State<'_, SettingsStore>) -> Result<Settings, IpcError> {
    Ok(store.get())
}

/// Replaces the settings file (atomic write: temp file + rename) and returns the canonical saved
/// value. Per SPEC-001 §2.5's "saved intent" rule, this command stores exactly what it's given —
/// it has no knowledge of runtime fallbacks (unsupported rate, clamped buffer size), so a caller
/// that only wants to persist the user's *chosen* value, not whatever got applied, is doing the
/// right thing by calling this with that chosen value.
#[tauri::command]
pub async fn settings_set(
    store: State<'_, SettingsStore>,
    documents: State<'_, crate::document::DocumentService>,
    settings: Settings,
) -> Result<Settings, IpcError> {
    let saved = store.set(settings)?;
    // T-301 (SPEC-004 §2.4): "Memory for audio" applies without a restart.
    documents.set_memory_budget_mib(saved.memory_budget_mib);
    Ok(saved)
}

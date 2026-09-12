use crate::ipc::dto::AppInfo;
use crate::ipc::error::IpcError;

/// Returns basic app identity info (name + version). Tauri command handlers are always `async`
/// so sync commands still run off the main thread (CLAUDE.md / ADR-003).
#[tauri::command]
pub async fn app_info() -> Result<AppInfo, IpcError> {
    Ok(AppInfo {
        name: "PowerVoice".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
    })
}

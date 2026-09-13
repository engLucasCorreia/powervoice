//! Recording commands (S1-04, SPEC-002 §2.1–§2.2, §2.7): arm, record start/stop, monitoring.
//! Thin: each forwards to the [`RecordingService`] (blocking work on `spawn_blocking`).

use tauri::State;

use crate::ipc::error::IpcError;
use crate::ipc::record_dto::RecordStateDto;
use crate::recording::RecordingService;
use crate::settings::{MonitorMode, SettingsStore};

async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, IpcError> + Send + 'static,
) -> Result<T, IpcError> {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| IpcError::internal(e.to_string()))?
}

/// The record panel state.
#[tauri::command]
pub async fn record_get(rec: State<'_, RecordingService>) -> Result<RecordStateDto, IpcError> {
    let rec = rec.inner().clone();
    blocking(move || rec.state()).await
}

/// Arms (opens the input, starts the meter) or disarms the input.
#[tauri::command]
pub async fn record_arm(
    rec: State<'_, RecordingService>,
    armed: bool,
) -> Result<RecordStateDto, IpcError> {
    let rec = rec.inner().clone();
    blocking(move || rec.arm(armed)).await
}

/// Starts a new recording. `replace`: the user confirmed replacing a document that has audio.
#[tauri::command]
pub async fn record_start(
    rec: State<'_, RecordingService>,
    replace: bool,
) -> Result<RecordStateDto, IpcError> {
    let rec = rec.inner().clone();
    blocking(move || rec.start(replace)).await
}

/// Stops the recording; the take becomes the document when it is committed.
#[tauri::command]
pub async fn record_stop(rec: State<'_, RecordingService>) -> Result<RecordStateDto, IpcError> {
    let rec = rec.inner().clone();
    blocking(move || rec.stop()).await
}

/// Sets the monitoring mode and saves it as the preference (SPEC-002 §2.7).
#[tauri::command]
pub async fn record_set_monitor(
    rec: State<'_, RecordingService>,
    settings: State<'_, SettingsStore>,
    mode: MonitorMode,
) -> Result<RecordStateDto, IpcError> {
    let service = rec.inner().clone();
    let state = blocking(move || service.set_monitor(mode)).await?;
    let mut saved = settings.get();
    if saved.monitor_mode != mode {
        saved.monitor_mode = mode;
        settings.set(saved)?;
    }
    Ok(state)
}

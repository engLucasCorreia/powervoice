//! Latency calibration commands (T-304, SPEC-022 §2.14, §4.9): `calibration_run { verify }` is a
//! job (`job_progress` + `calibration_result`), `calibration_cancel` aborts it. Thin: forwards to
//! the [`CalibrationService`].

use tauri::State;

use crate::audio::AudioEngine;
use crate::calibration::CalibrationService;
use crate::ipc::error::IpcError;
use crate::settings::SettingsStore;

async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, IpcError> + Send + 'static,
) -> Result<T, IpcError> {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| IpcError::internal(e.to_string()))?
}

/// Starts a calibration run; `verify`: re-measure with the current device setup's stored offset
/// applied (the result's `offset_ms` is then the residual). Returns the job id.
#[tauri::command]
pub async fn calibration_run(
    cal: State<'_, CalibrationService>,
    engine: State<'_, AudioEngine>,
    settings: State<'_, SettingsStore>,
    verify: bool,
) -> Result<u32, IpcError> {
    let cal = cal.inner().clone();
    let engine = engine.handle().clone();
    let saved = settings.get();
    blocking(move || {
        let offset = verify.then(|| crate::recording::current_offset_ms(&engine, &saved));
        cal.run(offset)
    })
    .await
}

/// Aborts the running calibration.
#[tauri::command]
pub async fn calibration_cancel(cal: State<'_, CalibrationService>) -> Result<(), IpcError> {
    let cal = cal.inner().clone();
    blocking(move || {
        cal.cancel();
        Ok(())
    })
    .await
}

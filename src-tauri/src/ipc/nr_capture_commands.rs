//! Capture Noise Print commands (S3-06, SPEC-014 §2.3). Thin: `NrCaptureService` does the work
//! (CLAUDE.md: no business logic in `src-tauri` commands).

use tauri::State;

use crate::ipc::error::IpcError;
use crate::ipc::nr_capture_dto::NrCaptureStartedDto;
use crate::nr_capture::{NrCaptureRequest, NrCaptureService};

/// Starts a Capture Noise Print job for `[start, end)` (document samples): resolves or inserts
/// the target Noise Reduction slot synchronously (the returned `slot` is authoritative), then
/// reads, renders and analyses the excerpt on its own thread. `job_progress` (kind `nr_capture`)
/// reports completion; warnings and errors arrive as `notice`s.
#[tauri::command]
pub async fn nr_capture_start(
    nr: State<'_, NrCaptureService>,
    hint_slot: Option<usize>,
    start: u64,
    end: u64,
) -> Result<NrCaptureStartedDto, IpcError> {
    let nr = nr.inner().clone();
    let (job_id, slot) = tauri::async_runtime::spawn_blocking(move || {
        nr.start_job(NrCaptureRequest {
            hint_slot,
            start,
            end,
        })
    })
    .await
    .map_err(|e| IpcError::internal(e.to_string()))??;
    Ok(NrCaptureStartedDto { job_id, slot })
}

/// Cancels a running capture job (best-effort; leaves any previous print unchanged). A no-op for
/// an unknown or already-finished job id.
#[tauri::command]
pub async fn nr_capture_cancel(
    nr: State<'_, NrCaptureService>,
    job_id: u32,
) -> Result<(), IpcError> {
    nr.cancel_job(job_id);
    Ok(())
}

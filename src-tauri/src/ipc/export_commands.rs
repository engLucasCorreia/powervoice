//! Export commands (S4-04): thin wrappers over [`ExportService`] (CLAUDE.md: no business logic
//! in `src-tauri` commands). `export_start` returns immediately with a job id; progress and the
//! final outcome arrive as `job_progress` events (ADR-003).

use tauri::State;

use crate::export::{ExportRequest, ExportService};
use crate::ipc::error::IpcError;
use crate::ipc::export_dto::{ExportFormatsDto, ExportRequestDto, ExportStartedDto};

/// MP3 availability (ADR-007 §4, D-013) for the export dialog's format list.
#[tauri::command]
pub async fn export_formats(
    export: State<'_, ExportService>,
) -> Result<ExportFormatsDto, IpcError> {
    Ok(ExportFormatsDto {
        mp3_available: export.mp3_available(),
    })
}

/// Starts an export job on its own thread; the returned `job_id` is echoed on every
/// `job_progress` event for it (`export_cancel` stops it).
#[tauri::command]
pub async fn export_start(
    export: State<'_, ExportService>,
    request: ExportRequestDto,
) -> Result<ExportStartedDto, IpcError> {
    let request: ExportRequest = request.try_into()?;
    let job_id = export.start_job(request)?;
    Ok(ExportStartedDto { job_id })
}

/// Cancels a running export job (best-effort, `job_progress` reports `JobState::Cancelled`); a
/// no-op for an unknown or already-finished job id.
#[tauri::command]
pub async fn export_cancel(export: State<'_, ExportService>, job_id: u32) -> Result<(), IpcError> {
    export.cancel_job(job_id);
    Ok(())
}

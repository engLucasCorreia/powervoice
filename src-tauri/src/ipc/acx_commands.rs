//! ACX check command (S4-03): a thin wrapper over [`LoudnessService::run_acx_check`] (CLAUDE.md:
//! no business logic in `src-tauri` commands) — synchronous request/response (unlike the analysis
//! job, this ticket asks for "Command `acx_check` → report DTO" directly: no job id, no
//! `job_progress`/cancel), run off the async runtime via `spawn_blocking` (mirrors
//! `document_commands::run_blocking`).

use tauri::State;

use crate::ipc::acx_dto::{AcxCheckReportDto, AcxCheckRequestDto};
use crate::ipc::document_commands::run_blocking;
use crate::ipc::error::IpcError;
use crate::loudness::LoudnessService;

/// Checks the whole document (processed or source, per `request.source`) against the ACX /
/// Audible submission rules (RMS, sample peak, noise floor) and returns one report.
#[tauri::command]
pub async fn acx_check(
    loudness: State<'_, LoudnessService>,
    request: AcxCheckRequestDto,
) -> Result<AcxCheckReportDto, IpcError> {
    let service = (*loudness).clone();
    let source = request.source.into();
    let report = run_blocking(move || service.run_acx_check(source)).await?;
    Ok(report.into())
}

//! Loudness commands (S4-01): thin wrapper over [`LoudnessService`] (CLAUDE.md: no business logic
//! in `src-tauri` commands). `loudness_analyze_start` returns immediately with a job id; progress
//! and the finished report arrive as `job_progress`/`loudness_report` events (mirrors
//! `export_start`, ADR-003).
//!
//! LUFS normalize (S4-01's other half) is `edit_normalize_lufs_start`/`_cancel`
//! (`ipc::normalize_commands`, H-09) — see that module and `crate::normalize::NormalizeService`.

use tauri::State;

use crate::ipc::error::IpcError;
use crate::ipc::loudness_dto::{LoudnessAnalyzeRequestDto, LoudnessAnalyzeStartedDto};
use crate::loudness::{LoudnessRequest, LoudnessService};

/// Starts a loudness analysis job on its own thread; the returned `job_id` is echoed on every
/// `job_progress`/`loudness_report` event for it (`loudness_analyze_cancel` stops it).
#[tauri::command]
pub async fn loudness_analyze_start(
    loudness: State<'_, LoudnessService>,
    request: LoudnessAnalyzeRequestDto,
) -> Result<LoudnessAnalyzeStartedDto, IpcError> {
    let range = match (request.start_sample, request.end_sample) {
        (Some(start), Some(end)) => Some((start, end)),
        _ => None,
    };
    let job_id = loudness.start_job(LoudnessRequest {
        range,
        source: request.source.into(),
    })?;
    Ok(LoudnessAnalyzeStartedDto { job_id })
}

/// Cancels a running loudness analysis job (best-effort, `job_progress` reports
/// `JobState::Cancelled`); a no-op for an unknown or already-finished job id.
#[tauri::command]
pub async fn loudness_analyze_cancel(
    loudness: State<'_, LoudnessService>,
    job_id: u32,
) -> Result<(), IpcError> {
    loudness.cancel_job(job_id);
    Ok(())
}

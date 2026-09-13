//! Loudness commands (S4-01): thin wrappers over [`LoudnessService`] and
//! [`DocumentService::edit_normalize_lufs`] (CLAUDE.md: no business logic in `src-tauri`
//! commands). `loudness_analyze_start` returns immediately with a job id; progress and the
//! finished report arrive as `job_progress`/`loudness_report` events (mirrors `export_start`,
//! ADR-003).

use tauri::{AppHandle, Runtime, State};

use crate::document::{DocumentService, NormalizeLufsNotice};
use crate::ipc::document_dto::EditResultDto;
use crate::ipc::error::IpcError;
use crate::ipc::events::{Notice, NoticeLevel, emit_notice};
use crate::ipc::loudness_dto::{
    LoudnessAnalyzeRequestDto, LoudnessAnalyzeStartedDto, normalize_lufs_notice_key,
};
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

/// LUFS-normalizes `[start_samples, end_samples)` (the frontend resolves "no selection" to the
/// whole file, same convention as `edit_normalize_peak`) to `target_lufs` integrated loudness —
/// one undo entry `history.normalize_lufs`, one click, no confirmation. A no-op (silent/near-
/// silent scope, or already at the target) posts `notice.normalize_silent`/
/// `notice.normalize_already` and changes nothing; an applied gain whose predicted true peak
/// exceeds −1 dBTP still applies and posts `notice.normalize_lufs_true_peak_ceiling`;
/// `error.normalize_non_finite` refuses a scope with a non-finite sample.
#[tauri::command]
pub async fn edit_normalize_lufs<R: Runtime>(
    app: AppHandle<R>,
    doc: State<'_, DocumentService>,
    start_samples: u64,
    end_samples: u64,
    target_lufs: f64,
) -> Result<EditResultDto, IpcError> {
    let service = (*doc).clone();
    let outcome = crate::ipc::document_commands::run_blocking(move || {
        service.edit_normalize_lufs(start_samples, end_samples, target_lufs)
    })
    .await?;
    crate::ipc::document_commands::after_edit(&app, &doc);
    if let Some(notice) = outcome.notice {
        let key = normalize_lufs_notice_key(notice);
        let mut event = Notice::toast(NoticeLevel::Info, key);
        if let NormalizeLufsNotice::TruePeakCeilingExceeded {
            predicted_true_peak_dbtp,
        } = notice
        {
            event = event.with_param("value", format!("{predicted_true_peak_dbtp:.1}"));
        }
        if let Err(error) = emit_notice(&app, event) {
            tracing::warn!(%error, "emitting a LUFS normalize notice failed");
        }
    }
    Ok(outcome.result.into())
}

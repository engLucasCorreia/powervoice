//! Normalize job commands (S2-02/S4-01, H-09). Thin: `NormalizeService` does the work (CLAUDE.md:
//! no business logic in `src-tauri` commands). Mirrors `nr_capture_commands.rs`'s `_start`/
//! `_cancel` pair, one for each of the peak/LUFS favorites and dialogs.

use tauri::State;

use crate::ipc::error::IpcError;
use crate::ipc::normalize_dto::NormalizeJobStartedDto;
use crate::normalize::NormalizeService;

/// Starts a peak-normalize job for `[start_samples, end_samples)` (the frontend resolves "no
/// selection" to the whole file, SPEC-010 §2.1), targeting `target_db` dBFS sample peak or
/// `target_pct` % of full scale (exactly one of the two, SPEC-010 §2.4) — one undo entry
/// `history.normalize`. `job_progress` (kind `normalize_peak`) reports progress/outcome; a
/// successful commit also arrives as `normalize_result`; a no-op posts
/// `notice.normalize_silent`/`notice.normalize_already`.
#[tauri::command]
pub async fn edit_normalize_peak_start(
    normalize: State<'_, NormalizeService>,
    start_samples: u64,
    end_samples: u64,
    target_db: Option<f64>,
    target_pct: Option<f64>,
) -> Result<NormalizeJobStartedDto, IpcError> {
    let normalize = normalize.inner().clone();
    let job_id = tauri::async_runtime::spawn_blocking(move || {
        normalize.start_peak_job(start_samples, end_samples, target_db, target_pct)
    })
    .await
    .map_err(|e| IpcError::internal(e.to_string()))??;
    Ok(NormalizeJobStartedDto { job_id })
}

/// Cancels a running peak-normalize job (best-effort, `job_progress` reports
/// `JobState::Cancelled`, leaving the document exactly as before — SPEC-010 §2.8); a no-op for an
/// unknown or already-finished job id.
#[tauri::command]
pub async fn edit_normalize_peak_cancel(
    normalize: State<'_, NormalizeService>,
    job_id: u32,
) -> Result<(), IpcError> {
    normalize.cancel_job(job_id);
    Ok(())
}

/// Starts a LUFS-normalize job for `[start_samples, end_samples)` targeting `target_lufs`
/// integrated loudness (mirrors [`edit_normalize_peak_start`], no % mode) — one undo entry
/// `history.normalize_lufs`. An applied gain whose predicted true peak exceeds −1 dBTP still
/// applies and posts `notice.normalize_lufs_true_peak_ceiling`.
#[tauri::command]
pub async fn edit_normalize_lufs_start(
    normalize: State<'_, NormalizeService>,
    start_samples: u64,
    end_samples: u64,
    target_lufs: f64,
) -> Result<NormalizeJobStartedDto, IpcError> {
    let normalize = normalize.inner().clone();
    let job_id = tauri::async_runtime::spawn_blocking(move || {
        normalize.start_lufs_job(start_samples, end_samples, target_lufs)
    })
    .await
    .map_err(|e| IpcError::internal(e.to_string()))??;
    Ok(NormalizeJobStartedDto { job_id })
}

/// Cancels a running LUFS-normalize job (mirrors [`edit_normalize_peak_cancel`]).
#[tauri::command]
pub async fn edit_normalize_lufs_cancel(
    normalize: State<'_, NormalizeService>,
    job_id: u32,
) -> Result<(), IpcError> {
    normalize.cancel_job(job_id);
    Ok(())
}

//! Bake rack job commands (T-602). Thin: `BakeService` does the work (CLAUDE.md: no business
//! logic in `src-tauri` commands). Mirrors `normalize_commands.rs`'s `_start`/`_cancel` pair.

use tauri::State;

use crate::bake::BakeService;
use crate::ipc::bake_dto::BakeJobStartedDto;
use crate::ipc::error::IpcError;

/// Starts a bake of `[start_samples, end_samples)` (the frontend resolves "no selection" to the
/// whole file) through the live rack — one undo entry `history.bake` that also restores the
/// pre-bake rack (SPEC-004 OD-4). Refused while recording, while another document job runs
/// (`error.document_busy`) and when the rack has no active slot (`error.bake.rack_inactive`).
/// `job_progress` (kind `bake`) reports progress and the outcome.
#[tauri::command]
pub async fn edit_bake_start(
    bake: State<'_, BakeService>,
    start_samples: u64,
    end_samples: u64,
) -> Result<BakeJobStartedDto, IpcError> {
    let bake = bake.inner().clone();
    let job_id =
        tauri::async_runtime::spawn_blocking(move || bake.start_job(start_samples, end_samples))
            .await
            .map_err(|e| IpcError::internal(e.to_string()))??;
    Ok(BakeJobStartedDto { job_id })
}

/// Cancels a running bake (best-effort; `job_progress` reports `Cancelled` and the document and
/// the rack stay exactly as they were). A no-op for an unknown or finished job id.
#[tauri::command]
pub async fn edit_bake_cancel(bake: State<'_, BakeService>, job_id: u32) -> Result<(), IpcError> {
    bake.cancel_job(job_id);
    Ok(())
}

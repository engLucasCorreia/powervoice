//! H-96 "belt and braces": one command, shared by every job kind, that answers a UI's recovery
//! query for a job it may have missed the terminal `job_progress` event for.

use tauri::State;

use crate::ipc::JobProgressDto;
use crate::job_status::JobStatusRegistry;

/// The last known status of `job_id` (any job kind — export, normalize peak/LUFS, bake), or
/// `null` for an id this app session never started (or started long enough ago to have been
/// evicted from the bounded recovery cache). The UI polls this on a timeout while a job looks
/// stuck in `running`, rather than trusting `job_progress` alone (ADR-003, MEMORY.md H-96).
#[tauri::command]
pub async fn job_status(
    status: State<'_, JobStatusRegistry>,
    job_id: u32,
) -> Result<Option<JobProgressDto>, crate::ipc::IpcError> {
    Ok(status.get(job_id))
}

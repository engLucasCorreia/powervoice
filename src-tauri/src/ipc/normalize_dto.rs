//! Normalize job DTOs (S2-02/S4-01, H-09, ADR-003).

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::ipc::document_dto::EditResultDto;
use crate::ipc::events::JobKind;

/// `edit_normalize_peak_start`/`edit_normalize_lufs_start`'s immediate result: the job id, echoed
/// on every `job_progress`/`normalize_result` event for it (`edit_normalize_peak_cancel`/
/// `edit_normalize_lufs_cancel` stop it).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "bindings.ts")]
pub struct NormalizeJobStartedDto {
    pub job_id: u32,
}

/// `normalize_result` event payload: the finished job's edit result (SPEC-010 §2.1's selection/
/// `audio_rev`, unchanged for a no-op). Emitted only once the job's `job_progress` reports `Done`
/// — a `Cancelled`/`Failed` job never touched the document, so there is nothing to report here
/// (its failure reason, if any, arrives as a `notice`).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "bindings.ts")]
pub struct NormalizeResultDto {
    pub job_id: u32,
    pub kind: JobKind,
    pub result: EditResultDto,
}

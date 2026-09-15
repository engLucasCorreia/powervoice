//! Bake rack job DTOs (T-602, ADR-003).

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// `edit_bake_start`'s immediate result: the job id, echoed on every `job_progress` event for it
/// (`edit_bake_cancel` stops it).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "bindings.ts")]
pub struct BakeJobStartedDto {
    pub job_id: u32,
}

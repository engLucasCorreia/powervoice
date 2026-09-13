//! Capture Noise Print DTOs (S3-06, SPEC-014 §2.3, ADR-003).

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// `nr_capture_start`'s immediate result: the job id (echoed on every `job_progress` event for
/// it) and the resolved target slot index — authoritative even when a slot was just inserted, so
/// the panel can show the spinner on the right one without waiting for `rack_changed`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "bindings.ts")]
pub struct NrCaptureStartedDto {
    pub job_id: u32,
    pub slot: usize,
}

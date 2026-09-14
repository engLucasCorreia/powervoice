//! T-304 (SPEC-022 §4.9): record-operation and calibration DTOs — `record_start_at`'s result, the
//! `record_phase` / `record_finished` / `calibration_result` events and the recording-offset
//! readout. Domain → DTO mapping only.

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use vox_dsp::calibration::{CalibrationReject, CalibrationResult};
use vox_engine::record::{CancelReason, RecordPhase, RecordPhaseInfo};
use vox_engine::record_op::RecordOpKind;

use crate::ipc::document_dto::EditResultDto;
use crate::ipc::record_dto::RecordStateDto;
use crate::settings::RecordOffsetSource;

/// What a Record press resolved to (SPEC-022 §2.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "bindings.ts", rename_all = "snake_case")]
pub enum RecordOpDto {
    New,
    Insert,
    Overwrite,
    Punch,
}

impl From<RecordOpKind> for RecordOpDto {
    fn from(k: RecordOpKind) -> Self {
        match k {
            RecordOpKind::New => Self::New,
            RecordOpKind::Insert => Self::Insert,
            RecordOpKind::Overwrite => Self::Overwrite,
            RecordOpKind::Punch => Self::Punch,
        }
    }
}

/// A record operation's phase (SPEC-022 §4.9).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "bindings.ts", rename_all = "snake_case")]
pub enum RecordPhaseKindDto {
    Preroll,
    Recording,
    Postroll,
    Committing,
}

/// `record_phase` event: sent at each phase change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "bindings.ts")]
pub struct RecordPhaseDto {
    pub take_id: u32,
    pub op: RecordOpDto,
    pub phase: RecordPhaseKindDto,
    /// Where the phase starts in the document (the record point for `recording`, `E` for
    /// `postroll`, the window end for `committing`).
    pub doc_pos_samples: u64,
    /// App-clock time of the change (`clock_now_ns` domain).
    pub app_ns: u64,
}

impl From<&RecordPhaseInfo> for RecordPhaseDto {
    fn from(i: &RecordPhaseInfo) -> Self {
        Self {
            take_id: i.take,
            op: i.kind.into(),
            phase: match i.phase {
                RecordPhase::PreRoll => RecordPhaseKindDto::Preroll,
                RecordPhase::Recording => RecordPhaseKindDto::Recording,
                RecordPhase::PostRoll => RecordPhaseKindDto::Postroll,
                RecordPhase::Committing => RecordPhaseKindDto::Committing,
            },
            doc_pos_samples: i.doc_pos_samples,
            app_ns: i.app_ns,
        }
    }
}

/// `record_start_at`'s result (SPEC-022 §4.9 `RecordStarted`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "bindings.ts")]
pub struct RecordStartedDto {
    pub take_id: u32,
    pub op: RecordOpDto,
    pub at_samples: u64,
    /// The punch end `E` (`null` for cursor recordings).
    pub end_samples: Option<u64>,
    pub preroll_samples: u64,
    pub postroll_samples: u64,
    /// The window opens at an aligned start (playback precedes it).
    pub aligned: bool,
    pub state: RecordStateDto,
}

/// Why an operation was cancelled (SPEC-022 §2.10).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "bindings.ts", rename_all = "snake_case")]
pub enum RecordCancelDto {
    User,
    InputLost,
    OutputLost,
    Aborted,
}

impl From<CancelReason> for RecordCancelDto {
    fn from(r: CancelReason) -> Self {
        match r {
            CancelReason::User => Self::User,
            CancelReason::InputLost => Self::InputLost,
            CancelReason::OutputLost => Self::OutputLost,
            CancelReason::Aborted => Self::Aborted,
        }
    }
}

/// `record_finished` event (SPEC-022 §4.9): committed with the edit result (selection and
/// playhead per §2.10), or cancelled with its reason.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "bindings.ts")]
pub struct RecordFinishedDto {
    pub take_id: u32,
    pub op: RecordOpDto,
    pub committed: bool,
    pub cancel_reason: Option<RecordCancelDto>,
    pub result: Option<EditResultDto>,
}

/// Why a calibration was rejected (SPEC-022 §2.14 step 3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "bindings.ts", rename_all = "snake_case")]
pub enum CalibrationRejectDto {
    NoSignal,
    LowConfidence,
}

/// `calibration_result` event (SPEC-022 §4.9), tagged with its job id.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "bindings.ts")]
pub struct CalibrationResultDto {
    pub job_id: u32,
    /// A Verify pass: `offset_ms` is the residual after compensation.
    pub verify: bool,
    pub offset_ms: f64,
    pub offset_samples: f64,
    pub device_rate_hz: u32,
    pub confidence: f64,
    pub reps_agreeing: u32,
    pub psr_median: f64,
    /// Highest recorded level (`null`: digital silence).
    pub peak_dbfs: Option<f64>,
    pub clipped: bool,
    pub weak: bool,
    pub accepted: bool,
    pub reason: Option<CalibrationRejectDto>,
}

impl CalibrationResultDto {
    pub fn new(job_id: u32, verify: bool, rate_hz: u32, r: &CalibrationResult) -> Self {
        let finite = |x: f64| if x.is_finite() { x } else { 1e6 };
        Self {
            job_id,
            verify,
            offset_ms: r.offset_ms,
            offset_samples: r.offset_samples,
            device_rate_hz: rate_hz,
            confidence: r.confidence,
            reps_agreeing: r.reps_agreeing,
            psr_median: finite(r.psr_median),
            peak_dbfs: r.peak_dbfs.is_finite().then_some(r.peak_dbfs),
            clipped: r.clipped,
            weak: r.weak,
            accepted: r.accepted,
            reason: r.reject.map(|x| match x {
                CalibrationReject::NoSignal => CalibrationRejectDto::NoSignal,
                CalibrationReject::LowConfidence => CalibrationRejectDto::LowConfidence,
            }),
        }
    }
}

/// The current device setup's recording offset (SPEC-022 §2.13 readout).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "bindings.ts")]
pub struct RecordOffsetDto {
    /// An input and an output device are configured (else nothing can be calibrated).
    pub available: bool,
    pub host: String,
    pub input_device: String,
    pub output_device: String,
    pub device_rate_hz: u32,
    /// The applied δ (0 when not calibrated).
    pub offset_ms: f64,
    /// `null`: not calibrated.
    pub source: Option<RecordOffsetSource>,
    pub updated_unix_ms: Option<u64>,
    pub confidence: Option<f64>,
    /// The buffer size it was measured at.
    pub buffer_frames: Option<u32>,
    /// The current output buffer size (a mismatch shows the recalibrate hint).
    pub current_buffer_frames: Option<u32>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// §4.9 names on the wire: snake_case phases and operations, `null` for a silent peak.
    #[test]
    fn wire_shapes() {
        let info = RecordPhaseInfo {
            take: 3,
            kind: RecordOpKind::Punch,
            phase: RecordPhase::PreRoll,
            doc_pos_samples: 42,
            app_ns: 7,
        };
        let json = serde_json::to_string(&RecordPhaseDto::from(&info)).unwrap();
        assert!(json.contains("\"phase\":\"preroll\""), "{json}");
        assert!(json.contains("\"op\":\"punch\""), "{json}");
        let r = CalibrationResult {
            offset_samples: 0.0,
            offset_ms: 0.0,
            confidence: 0.0,
            reps_agreeing: 0,
            psr_median: f64::INFINITY,
            peak_dbfs: f64::NEG_INFINITY,
            clipped: false,
            weak: false,
            accepted: false,
            reject: Some(CalibrationReject::NoSignal),
            reps: Vec::new(),
        };
        let dto = CalibrationResultDto::new(1, false, 48_000, &r);
        assert_eq!(dto.peak_dbfs, None);
        assert!(
            (dto.psr_median - 1e6).abs() < 1e-9,
            "a finite stand-in for ∞"
        );
        let json = serde_json::to_string(&dto).unwrap();
        assert!(json.contains("\"reason\":\"no_signal\""), "{json}");
    }
}

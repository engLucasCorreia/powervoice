//! ACX check DTOs (S4-03): the `acx_check` command's request/report shapes — one row per ACX rule
//! (RMS, sample peak, noise floor; docs/references.md) with the measured value and pass/fail
//! status, over the processed or source signal (same `source` toggle convention as
//! `loudness_analyze_start`, S4-01), always for the whole document (ACX submissions are whole
//! chapters, not selections).

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use vox_dsp::acx::{AcxReport, AcxRuleReport, AcxRuleStatus};

use crate::ipc::loudness_dto::LoudnessSourceDto;

/// `acx_check`'s argument.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "bindings.ts")]
pub struct AcxCheckRequestDto {
    pub source: LoudnessSourceDto,
}

/// Mirrors [`AcxRuleStatus`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "bindings.ts", rename_all = "snake_case")]
pub enum AcxRuleStatusDto {
    Pass,
    TooLow,
    TooHigh,
    Invalid,
    TooShort,
}

impl From<AcxRuleStatus> for AcxRuleStatusDto {
    fn from(status: AcxRuleStatus) -> Self {
        match status {
            AcxRuleStatus::Pass => AcxRuleStatusDto::Pass,
            AcxRuleStatus::TooLow => AcxRuleStatusDto::TooLow,
            AcxRuleStatus::TooHigh => AcxRuleStatusDto::TooHigh,
            AcxRuleStatus::Invalid => AcxRuleStatusDto::Invalid,
            AcxRuleStatus::TooShort => AcxRuleStatusDto::TooShort,
        }
    }
}

/// One rule's row in the ACX Check table: the measured value (`None` only for
/// [`AcxRuleStatusDto::TooShort`]) and its pass/fail status.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "bindings.ts")]
pub struct AcxRuleDto {
    pub measured_db: Option<f64>,
    pub status: AcxRuleStatusDto,
}

impl From<AcxRuleReport> for AcxRuleDto {
    fn from(rule: AcxRuleReport) -> Self {
        Self {
            measured_db: rule.measured_db,
            status: rule.status.into(),
        }
    }
}

/// `acx_check`'s result: whole-document pass/fail (`passes`, every rule must pass) plus each
/// rule's row.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "bindings.ts")]
pub struct AcxCheckReportDto {
    pub passes: bool,
    pub rms: AcxRuleDto,
    pub peak: AcxRuleDto,
    pub noise_floor: AcxRuleDto,
}

impl From<AcxReport> for AcxCheckReportDto {
    fn from(report: AcxReport) -> Self {
        Self {
            passes: report.passes(),
            rms: report.rms.into(),
            peak: report.peak.into(),
            noise_floor: report.noise_floor.into(),
        }
    }
}

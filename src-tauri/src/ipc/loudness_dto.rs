//! Loudness DTOs (S4-01): the analysis job's request/report shapes and the LUFS normalize result.
//!
//! `-inf`/`NaN` fields serialize as JSON `null` (serde_json's own float behaviour) — the same
//! convention `powervoice-cli analyze --json` already uses for these exact values (T-006,
//! MEMORY.md); the frontend checks `Number.isFinite` before formatting, like the output meter
//! already does for silence.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::document::NormalizeLufsNotice;
use crate::loudness::LoudnessSource;
use vox_dsp::loudness::LoudnessReport;

/// `loudness_analyze_start`'s argument: the scope (`None` = whole file) and which signal to
/// measure (ticket: "processed by default, with a source toggle").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "bindings.ts")]
pub struct LoudnessAnalyzeRequestDto {
    pub start_sample: Option<u64>,
    pub end_sample: Option<u64>,
    pub source: LoudnessSourceDto,
}

/// Mirrors [`LoudnessSource`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "bindings.ts", rename_all = "snake_case")]
pub enum LoudnessSourceDto {
    Processed,
    Source,
}

impl From<LoudnessSourceDto> for LoudnessSource {
    fn from(dto: LoudnessSourceDto) -> Self {
        match dto {
            LoudnessSourceDto::Processed => LoudnessSource::Processed,
            LoudnessSourceDto::Source => LoudnessSource::Source,
        }
    }
}

/// `loudness_analyze_start`'s result: the job runs in the background, reporting `job_progress`
/// (`JobKind::LoudnessAnalyze`) tagged with this `job_id`, then a `loudness_report` event once
/// done (`loudness_analyze_cancel(job_id)` stops it early).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "bindings.ts")]
pub struct LoudnessAnalyzeStartedDto {
    pub job_id: u32,
}

/// The `loudness_report` event payload: `job_id` ties it to the `job_progress` events of the same
/// job (S4-01 ticket's report shape: I / S-max / M-max / LRA / sample peak / TP).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "bindings.ts")]
pub struct LoudnessReportDto {
    pub job_id: u32,
    pub integrated_lufs: f64,
    pub max_momentary_lufs: Option<f64>,
    pub max_short_term_lufs: Option<f64>,
    pub lra_lu: f64,
    pub sample_peak_dbfs: f64,
    pub true_peak_dbtp: f64,
}

impl LoudnessReportDto {
    pub fn from_report(job_id: u32, report: LoudnessReport) -> Self {
        Self {
            job_id,
            integrated_lufs: report.integrated_lufs,
            max_momentary_lufs: report.max_momentary_lufs,
            max_short_term_lufs: report.max_short_term_lufs,
            lra_lu: report.lra_lu,
            sample_peak_dbfs: report.sample_peak_dbfs,
            true_peak_dbtp: report.true_peak_dbtp,
        }
    }
}

/// `edit_normalize_lufs`'s notice (mirrors `NormalizeNotice`'s DTO-less handling: the command
/// handler turns it straight into a `notice` toast — this type only exists so `document_dto`-style
/// call sites don't need to duplicate the match). Not itself a DTO: no `TS` derive, since it never
/// crosses the IPC boundary as JSON (the command emits the equivalent i18n key/params instead).
pub fn normalize_lufs_notice_key(notice: NormalizeLufsNotice) -> &'static str {
    match notice {
        NormalizeLufsNotice::Silent => "notice.normalize_silent",
        NormalizeLufsNotice::AlreadyNormalized => "notice.normalize_already",
        NormalizeLufsNotice::TruePeakCeilingExceeded { .. } => {
            "notice.normalize_lufs_true_peak_ceiling"
        }
    }
}

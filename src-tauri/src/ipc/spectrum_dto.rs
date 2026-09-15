//! Long-term average spectrum DTOs (H-42, SPEC-007 §8.5): the job's request and its
//! `spectrum_report` event. The curves are not here — they're bulk data, fetched as binary
//! `VXLT` frames (`spectrum_analyze_curve`).

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::ipc::analyzer_dto::{SpectrumWindowDto, VoiceReportDto};
use crate::ipc::loudness_dto::LoudnessSourceDto;
use crate::loudness::LoudnessSource;

impl From<LoudnessSource> for LoudnessSourceDto {
    fn from(source: LoudnessSource) -> Self {
        match source {
            LoudnessSource::Processed => LoudnessSourceDto::Processed,
            LoudnessSource::Source => LoudnessSourceDto::Source,
        }
    }
}

/// `spectrum_analyze_start`'s argument.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "bindings.ts")]
pub struct SpectrumAnalyzeRequestDto {
    /// `[start, end)`; both `null` = the whole document.
    pub start_sample: Option<u64>,
    pub end_sample: Option<u64>,
    /// The signals to analyse, in order — `["source", "processed"]` for the comparison.
    pub sources: Vec<LoudnessSourceDto>,
    /// A power of two, 1 024 … 32 768.
    pub fft_size: u32,
    pub window: SpectrumWindowDto,
}

/// `spectrum_analyze_start`'s result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "bindings.ts")]
pub struct SpectrumAnalyzeStartedDto {
    pub job_id: u32,
}

/// One analysed signal of a `spectrum_report`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "bindings.ts")]
pub struct SpectrumResultDto {
    pub source: LoudnessSourceDto,
    /// Frames averaged into the curve.
    pub frames: u64,
    /// Whether the `VXLT` curve carries a room-tone (quiet-frame) spectrum too.
    pub has_noise: bool,
    pub report: VoiceReportDto,
}

/// The `spectrum_report` event: a finished long-term average job. Fetch result `i`'s curve with
/// `spectrum_analyze_curve(job_id, i)`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "bindings.ts")]
pub struct SpectrumReportDto {
    pub job_id: u32,
    pub sample_rate_hz: u32,
    pub fft_size: u32,
    pub window: SpectrumWindowDto,
    pub start_sample: u64,
    pub end_sample: u64,
    pub results: Vec<SpectrumResultDto>,
}

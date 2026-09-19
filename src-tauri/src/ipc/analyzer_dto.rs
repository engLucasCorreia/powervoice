//! Live output analyzer DTOs (T-208, SPEC-007 §2.9/§4.8–§4.9): the `AnalyzerResponseDto` enum
//! for `analyzer_subscribe`/`analyzer_set_response`, and the Rust-generated `VXSA` fixture for
//! the TS decoder test (ADR-003 §4, ADR-003 Amendment 4).

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use vox_dsp::diagnostics::{VoiceReport, WindowKind};
use vox_engine::{AnalyzerResponse, InspectorConfig};

/// Analyzer averaging response (Fast/Medium/Slow, SPEC-007 §2.9), exponential averaging in the
/// power domain with τ = 50 / 150 / 500 ms.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "bindings.ts", rename_all = "snake_case")]
pub enum AnalyzerResponseDto {
    Fast,
    Medium,
    Slow,
}

impl From<AnalyzerResponseDto> for AnalyzerResponse {
    fn from(d: AnalyzerResponseDto) -> Self {
        match d {
            AnalyzerResponseDto::Fast => AnalyzerResponse::Fast,
            AnalyzerResponseDto::Medium => AnalyzerResponse::Medium,
            AnalyzerResponseDto::Slow => AnalyzerResponse::Slow,
        }
    }
}

impl From<AnalyzerResponse> for AnalyzerResponseDto {
    fn from(r: AnalyzerResponse) -> Self {
        match r {
            AnalyzerResponse::Fast => AnalyzerResponseDto::Fast,
            AnalyzerResponse::Medium => AnalyzerResponseDto::Medium,
            AnalyzerResponse::Slow => AnalyzerResponseDto::Slow,
        }
    }
}

/// H-42 (SPEC-007 §8.3): a spectral-analysis window for the Spectrum Inspector and the
/// long-term average job. Mirrors [`WindowKind`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "bindings.ts", rename_all = "snake_case")]
pub enum SpectrumWindowDto {
    Hann,
    BlackmanHarris,
    FlatTop,
    Rectangular,
}

impl From<SpectrumWindowDto> for WindowKind {
    fn from(d: SpectrumWindowDto) -> Self {
        match d {
            SpectrumWindowDto::Hann => WindowKind::Hann,
            SpectrumWindowDto::BlackmanHarris => WindowKind::BlackmanHarris,
            SpectrumWindowDto::FlatTop => WindowKind::FlatTop,
            SpectrumWindowDto::Rectangular => WindowKind::Rectangular,
        }
    }
}

impl From<WindowKind> for SpectrumWindowDto {
    fn from(w: WindowKind) -> Self {
        match w {
            WindowKind::Hann => SpectrumWindowDto::Hann,
            WindowKind::BlackmanHarris => SpectrumWindowDto::BlackmanHarris,
            WindowKind::FlatTop => SpectrumWindowDto::FlatTop,
            WindowKind::Rectangular => SpectrumWindowDto::Rectangular,
        }
    }
}

/// H-42: `analyzer_inspector_subscribe`/`analyzer_inspector_configure`'s settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "bindings.ts")]
pub struct InspectorConfigDto {
    /// A power of two, 1 024 … 32 768.
    pub fft_size: u32,
    pub window: SpectrumWindowDto,
    pub response: AnalyzerResponseDto,
}

impl From<InspectorConfigDto> for InspectorConfig {
    fn from(d: InspectorConfigDto) -> Self {
        Self {
            fft_size: d.fft_size,
            window: d.window.into(),
            response: d.response.into(),
        }
    }
}

/// F0 statistics (Hz), SPEC-007 §8.6.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "bindings.ts")]
pub struct F0StatsDto {
    /// Live only: the last 0.3 s of voiced frames; `null` between phrases and offline.
    pub current_hz: Option<f64>,
    pub median_hz: f64,
    /// 10th percentile.
    pub low_hz: f64,
    /// 90th percentile.
    pub high_hz: f64,
    /// 0 … 1.
    pub voiced_fraction: f64,
    /// H-91: `1 − median aperiodicity` of the voiced frames, 0 … 1 — how periodic the voice
    /// actually was, so a firm reading can be told from a breathy guess.
    pub confidence: f64,
    /// H-91: share of the voiced frames that were an octave off the robust centre and were
    /// folded onto it, 0 … 1.
    pub octave_corrected: f64,
}

/// Per-octave densities relative to the 1 kHz octave (dB), SPEC-007 §8.7.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "bindings.ts")]
pub struct ToneBalanceDto {
    pub mud_db: f64,
    pub presence_db: Option<f64>,
    pub air_db: Option<f64>,
}

/// SPEC-007 §8.7.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "bindings.ts")]
pub struct SibilanceDto {
    /// 4–10 kHz relative to the overall level (dB).
    pub ratio_db: f64,
    /// The de-esser target (Hz).
    pub centre_hz: f64,
}

/// SPEC-007 §8.7.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "bindings.ts")]
pub struct HumDto {
    /// 50 or 60.
    pub mains_hz: f64,
    /// The harmonics that stand out (1 = the fundamental), ascending.
    pub harmonics: Vec<u32>,
    pub strongest_hz: f64,
    pub prominence_db: f64,
    pub level_db: f64,
}

/// The voice diagnostics (SPEC-007 §8.6–§8.8), live (`analyzer_voice_subscribe`) or from the
/// long-term average job (`spectrum_report`). `null` = not measurable.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "bindings.ts")]
pub struct VoiceReportDto {
    pub f0: Option<F0StatsDto>,
    pub tone: Option<ToneBalanceDto>,
    pub sibilance: Option<SibilanceDto>,
    pub hum: Option<HumDto>,
    pub rumble_db: Option<f64>,
    pub noise_floor_dbfs: Option<f64>,
    pub active_level_dbfs: Option<f64>,
    pub snr_db: Option<f64>,
    /// Seconds of non-silent audio the statistics cover.
    pub span_s: f64,
}

impl From<&VoiceReport> for VoiceReportDto {
    fn from(r: &VoiceReport) -> Self {
        Self {
            f0: r.f0.map(|f| F0StatsDto {
                current_hz: f.current_hz,
                median_hz: f.median_hz,
                low_hz: f.low_hz,
                high_hz: f.high_hz,
                voiced_fraction: f.voiced_fraction,
                confidence: f.confidence,
                octave_corrected: f.octave_corrected,
            }),
            tone: r.tone.map(|t| ToneBalanceDto {
                mud_db: t.mud_db,
                presence_db: t.presence_db,
                air_db: t.air_db,
            }),
            sibilance: r.sibilance.map(|s| SibilanceDto {
                ratio_db: s.ratio_db,
                centre_hz: s.centre_hz,
            }),
            hum: r.hum.map(|h| HumDto {
                mains_hz: h.mains_hz,
                harmonics: (1u32..=32)
                    .filter(|k| h.harmonics & (1 << (k - 1)) != 0)
                    .collect(),
                strongest_hz: h.strongest_hz,
                prominence_db: h.prominence_db,
                level_db: h.level_db,
            }),
            rumble_db: r.rumble_db,
            noise_floor_dbfs: r.noise_floor_dbfs,
            active_level_dbfs: r.active_level_dbfs,
            snr_db: r.snr_db,
            span_s: r.span_s,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use vox_dsp::diagnostics::WindowKind;
    use vox_engine::{AnalyzerFrame, AnalyzerResponse, InspectorFrame};

    /// Writes the golden `VXSA` frame for the TS decoder test (SPEC-007 §4.9: generated by the
    /// `export_bindings` run of `just gen-types`, checked by `just check-types`).
    #[test]
    fn export_bindings_vxsa_fixture() {
        let f = AnalyzerFrame {
            seq: 11,
            reset: true,
            dropped: false,
            silent: false,
            frame_time_ns: 555_555_555_000,
            sample_rate_hz: 48_000,
            fft_size: 8192,
            band_count: 4,
            response: AnalyzerResponse::Medium,
            levels_db: vec![-120.0, -20.375, 0.0, f32::NEG_INFINITY],
        };
        let hex: String = f.encode().iter().map(|b| format!("{b:02x}")).collect();
        let ts = format!(
            "// Generated by the `export_bindings_vxsa_fixture` test (src-tauri, `just gen-types`). Do not edit.\n\
             \n\
             /** A `VXSA` frame encoded by Rust (`AnalyzerFrame::encode`) for the TS decoder contract test. */\n\
             export const VXSA_FIXTURE_HEX =\n  \"{hex}\";\n\
             \n\
             /** The field values encoded in `VXSA_FIXTURE_HEX`. */\n\
             export const VXSA_FIXTURE_FIELDS = {{\n  \
             seq: 11,\n  reset: true,\n  dropped: false,\n  silent: false,\n  \
             frameTimeNs: 555555555000,\n  sampleRateHz: 48000,\n  fftSize: 8192,\n  \
             f0Hz: 20,\n  bandsPerOctave: 24,\n  bandCount: 4,\n  response: 1,\n  \
             levelsDb: [-120, -20.375, 0, -Infinity],\n}} as const;\n",
        );
        let dir = std::env::var_os("TS_RS_EXPORT_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("bindings"));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("vxsa_fixture.ts"), ts).unwrap();
    }

    /// H-42: the golden `VXIS` Spectrum Inspector frame for the TS decoder test (SPEC-007 §8.9).
    #[test]
    fn export_bindings_vxis_fixture() {
        let f = InspectorFrame {
            seq: 7,
            reset: true,
            silent: false,
            sample_rate_hz: 48_000,
            fft_size: 1024,
            window: WindowKind::FlatTop,
            response: AnalyzerResponse::Slow,
            levels_db: vec![-100.0, -6.5, 0.0, f32::NEG_INFINITY],
        };
        let hex: String = f.encode().iter().map(|b| format!("{b:02x}")).collect();
        let ts = format!(
            "// Generated by the `export_bindings_vxis_fixture` test (src-tauri, `just gen-types`). Do not edit.\n\
             \n\
             /** A `VXIS` frame encoded by Rust (`InspectorFrame::encode`) for the TS decoder contract test. */\n\
             export const VXIS_FIXTURE_HEX =\n  \"{hex}\";\n\
             \n\
             /** The field values encoded in `VXIS_FIXTURE_HEX`. */\n\
             export const VXIS_FIXTURE_FIELDS = {{\n  \
             seq: 7,\n  reset: true,\n  silent: false,\n  sampleRateHz: 48000,\n  \
             fftSize: 1024,\n  window: \"flat_top\",\n  response: \"slow\",\n  \
             levelsDb: [-100, -6.5, 0, -Infinity],\n}} as const;\n",
        );
        let dir = std::env::var_os("TS_RS_EXPORT_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("bindings"));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("vxis_fixture.ts"), ts).unwrap();
    }
}

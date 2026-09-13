//! Export DTOs (S4-04): the dialog's format/rate/range choices, MP3 availability, and the
//! `export_start` result (a `job_id` — progress arrives through `job_progress`, ADR-003).

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::export::{ExportFormat, ExportRequest};
use crate::settings::BitDepth;

/// `export_formats`' result: whether MP3 is available on this system (ADR-007 §4, D-013). WAV and
/// FLAC are always available.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "bindings.ts")]
pub struct ExportFormatsDto {
    pub mp3_available: bool,
}

/// MP3 encoder settings (mirrors [`vox_io::Mp3Settings`]; ticket S4-02: CBR 128-320 kbps or VBR
/// V0-V4). The ACX preset (PROMPT §3.5) is `Cbr { kbps: 192 }` at 44.1 kHz.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(export, export_to = "bindings.ts", rename_all = "snake_case")]
pub enum Mp3SettingsDto {
    Cbr { kbps: u32 },
    Vbr { quality: u8 },
}

impl From<Mp3SettingsDto> for vox_io::Mp3Settings {
    fn from(dto: Mp3SettingsDto) -> Self {
        match dto {
            Mp3SettingsDto::Cbr { kbps } => vox_io::Mp3Settings::Cbr { kbps },
            Mp3SettingsDto::Vbr { quality } => vox_io::Mp3Settings::Vbr { quality },
        }
    }
}

/// Export output format and its per-format settings. FLAC's `bits` rejects `"32f"` (FLAC has no
/// float sample format, SPEC-005 §2.11) with `error.export.flac_no_float`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(export, export_to = "bindings.ts", rename_all = "snake_case")]
pub enum ExportFormatDto {
    Wav { bits: BitDepth },
    Flac { bits: BitDepth },
    Mp3 { settings: Mp3SettingsDto },
}

/// `[start_sample, end_sample)` of the export (a selection); omitted in the request = whole file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "bindings.ts")]
pub struct ExportRangeDto {
    pub start_sample: u64,
    pub end_sample: u64,
}

/// `export_start`'s argument.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "bindings.ts")]
pub struct ExportRequestDto {
    pub path: String,
    pub format: ExportFormatDto,
    pub sample_rate_hz: u32,
    pub range: Option<ExportRangeDto>,
}

/// Maps the DTO to the domain [`ExportRequest`], or `Err` for FLAC + 32-bit float
/// (`error.export.flac_no_float`, the one combination the type system doesn't already rule out).
impl TryFrom<ExportRequestDto> for ExportRequest {
    type Error = crate::ipc::IpcError;

    fn try_from(dto: ExportRequestDto) -> Result<Self, Self::Error> {
        let format = match dto.format {
            ExportFormatDto::Wav { bits } => ExportFormat::Wav(bits.into()),
            ExportFormatDto::Flac {
                bits: BitDepth::Bit32Float,
            } => {
                return Err(crate::ipc::IpcError::new(
                    crate::ipc::IpcErrorCode::InvalidArgument,
                    "error.export.flac_no_float",
                ));
            }
            ExportFormatDto::Flac { bits } => ExportFormat::Flac(match bits {
                BitDepth::Bit16 => vox_io::FlacBitDepth::Int16,
                BitDepth::Bit24 => vox_io::FlacBitDepth::Int24,
                BitDepth::Bit32Float => unreachable!("matched above"),
            }),
            ExportFormatDto::Mp3 { settings } => ExportFormat::Mp3(settings.into()),
        };
        Ok(ExportRequest {
            path: std::path::PathBuf::from(dto.path),
            format,
            sample_rate_hz: dto.sample_rate_hz,
            range: dto.range.map(|r| (r.start_sample, r.end_sample)),
        })
    }
}

/// `export_start`'s result: the export runs in the background, reporting `job_progress` events
/// tagged with this `job_id` (`export_cancel(job_id)` stops it).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "bindings.ts")]
pub struct ExportStartedDto {
    pub job_id: u32,
}

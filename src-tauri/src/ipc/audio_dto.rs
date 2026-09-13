//! Audio DTOs (S1-01): transport state and the Audio Devices view, the engine ↔ DTO mapping
//! (incl. T-104's `DevicePrefsDto` ↔ the engine's `DevicePrefs`), device notices → i18n notices,
//! and the Rust-generated `VXTM` fixture for the TS decoder test (ADR-003 §4).

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use vox_engine::device_state::DeviceStatus;
use vox_engine::devices::{DeviceNotice, offered_buffer_sizes, offered_sample_rates};
use vox_engine::{BufferRequest, DeviceInfo, DevicePrefs, DevicesView, Direction, TransportState};

use crate::ipc::events::{Notice, NoticeLevel};
use crate::settings::{DevicePrefsDto, default_host};

/// Transport state (`transport_state` event, transport command results). While playing, the
/// moving playhead comes from `VXTM` telemetry; `playhead_samples` is then where the pass began.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "bindings.ts")]
pub struct TransportStateDto {
    pub playing: bool,
    pub playhead_samples: u64,
    pub play_start_samples: u64,
    pub doc_len_samples: u64,
    pub doc_rate_hz: u32,
    /// A document is loaded and an output device is open.
    pub can_play: bool,
}

impl From<&TransportState> for TransportStateDto {
    fn from(s: &TransportState) -> Self {
        Self {
            playing: s.playing,
            playhead_samples: s.playhead_samples,
            play_start_samples: s.play_start_samples,
            doc_len_samples: s.doc_len_samples,
            doc_rate_hz: s.doc_rate_hz,
            can_play: s.can_play,
        }
    }
}

/// Output status dot (SPEC-001 §2.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "bindings.ts", rename_all = "snake_case")]
pub enum DeviceStatusDto {
    NotSelected,
    Healthy,
    Fallback,
    Lost,
}

impl From<DeviceStatus> for DeviceStatusDto {
    fn from(s: DeviceStatus) -> Self {
        match s {
            DeviceStatus::NotSelected => Self::NotSelected,
            DeviceStatus::Healthy => Self::Healthy,
            DeviceStatus::Fallback => Self::Fallback,
            DeviceStatus::Lost => Self::Lost,
        }
    }
}

/// One device of the selected host.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "bindings.ts")]
pub struct DeviceDto {
    /// Unique name (the key saved in the prefs).
    pub name: String,
    /// Display name.
    pub base_name: String,
    pub input: bool,
    pub output: bool,
    /// Input channels (S1-04 channel dropdown; 0 while unknown).
    pub input_channels: u16,
    /// A "follow the system default" pseudo-device.
    pub system_default: bool,
    /// Its capture side monitors an output (hidden from the input list).
    pub input_is_monitor: bool,
    /// SPEC-001 §2.1 common rates the output supports (empty while unknown).
    pub output_rates_hz: Vec<u32>,
    /// SPEC-001 §2.1 common buffer sizes within the output's range ("Auto" is implicit).
    pub output_buffer_sizes: Vec<u32>,
}

impl From<&DeviceInfo> for DeviceDto {
    fn from(d: &DeviceInfo) -> Self {
        let caps = d.caps(Direction::Output);
        Self {
            name: d.name.clone(),
            base_name: d.base_name.clone(),
            input: d.supports(Direction::Input),
            output: d.supports(Direction::Output),
            input_channels: d.caps(Direction::Input).map_or(0, |c| c.max_channels),
            system_default: d.system_default,
            input_is_monitor: d.input_is_monitor,
            output_rates_hz: caps.map(offered_sample_rates).unwrap_or_default(),
            output_buffer_sizes: caps.map(offered_buffer_sizes).unwrap_or_default(),
        }
    }
}

/// The Audio Devices dialog's data (`devices_list`, `devices_select`, `devices_changed`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "bindings.ts")]
pub struct DevicesDto {
    pub hosts: Vec<String>,
    pub host: Option<String>,
    pub devices: Vec<DeviceDto>,
    pub default_input: Option<String>,
    pub default_output: Option<String>,
    /// The saved intent.
    pub prefs: DevicePrefsDto,
    /// Applied output device, rate and buffer (`None` = Auto / not open).
    pub output_device: Option<String>,
    pub output_rate_hz: Option<u32>,
    pub output_buffer_frames: Option<u32>,
    pub output_status: DeviceStatusDto,
    /// The configured input device and its status dot (S1-04).
    pub input_device: Option<String>,
    pub input_status: DeviceStatusDto,
}

impl From<&DevicesView> for DevicesDto {
    fn from(v: &DevicesView) -> Self {
        Self {
            hosts: v.hosts.iter().map(|h| h.as_str().to_owned()).collect(),
            host: v.host.map(|h| h.as_str().to_owned()),
            devices: v.snapshot.devices.iter().map(DeviceDto::from).collect(),
            default_input: v.snapshot.default_input.clone(),
            default_output: v.snapshot.default_output.clone(),
            prefs: DevicePrefsDto::from(&v.prefs),
            output_device: v.output_device.clone(),
            output_rate_hz: v.output_rate_hz,
            output_buffer_frames: match v.output_buffer {
                Some(BufferRequest::Frames(n)) => Some(n),
                _ => None,
            },
            output_status: v.output_status.into(),
            input_device: v.input_device.clone(),
            input_status: v.input_status.into(),
        }
    }
}

impl From<&DevicePrefsDto> for DevicePrefs {
    fn from(d: &DevicePrefsDto) -> Self {
        Self {
            host: d.host.parse().ok(),
            input_device: d.input_device.clone(),
            input_channel: u16::try_from(d.input_channel).unwrap_or(1).max(1),
            output_device: d.output_device.clone(),
            sample_rate_hz: d.sample_rate_hz,
            buffer_size: d
                .buffer_size_frames
                .map_or(BufferRequest::Auto, BufferRequest::Frames),
        }
    }
}

impl From<&DevicePrefs> for DevicePrefsDto {
    fn from(p: &DevicePrefs) -> Self {
        Self {
            host: p.host.map_or_else(default_host, |h| h.as_str().to_owned()),
            input_device: p.input_device.clone(),
            input_channel: u32::from(p.input_channel),
            output_device: p.output_device.clone(),
            sample_rate_hz: p.sample_rate_hz,
            buffer_size_frames: match p.buffer_size {
                BufferRequest::Frames(n) => Some(n),
                BufferRequest::Auto => None,
            },
        }
    }
}

fn dir_str(d: Direction) -> &'static str {
    match d {
        Direction::Input => "input",
        Direction::Output => "output",
    }
}

/// Device notice → i18n notice (SPEC-001 §2.2–§2.4). Lost/reconnected share one banner id per
/// direction, so "reconnected" replaces "disconnected".
pub fn notice_from_device(n: &DeviceNotice) -> Notice {
    use NoticeLevel::{Error, Info, Warning};
    match n {
        DeviceNotice::RateFallback {
            device,
            requested_hz,
            applied_hz,
        } => Notice::toast(Warning, "notice.device.rate_fallback")
            .with_param("device", device)
            .with_param("requested", requested_hz.to_string())
            .with_param("applied", applied_hz.to_string()),
        DeviceNotice::BufferFallback {
            device,
            requested_frames,
            applied_frames,
        } => Notice::toast(Warning, "notice.device.buffer_fallback")
            .with_param("device", device)
            .with_param("requested", requested_frames.to_string())
            .with_param("applied", applied_frames.to_string()),
        DeviceNotice::ChannelFallback {
            device,
            requested,
            applied,
        } => Notice::toast(Warning, "notice.device.channel_fallback")
            .with_param("device", device)
            .with_param("requested", requested.to_string())
            .with_param("applied", applied.to_string()),
        DeviceNotice::DeviceNotFound {
            direction,
            saved,
            using,
        } => match using {
            Some(using) => Notice::toast(
                Warning,
                format!("notice.device.not_found.{}", dir_str(*direction)),
            )
            .with_param("device", saved)
            .with_param("using", using),
            None => Notice::toast(Warning, "notice.device.not_found_no_default")
                .with_param("device", saved),
        },
        DeviceNotice::HostNotFound { saved, using } => {
            Notice::toast(Warning, "notice.device.host_not_found")
                .with_param("host", saved.as_str())
                .with_param("using", using.as_str())
        }
        DeviceNotice::DeviceLost {
            direction, device, ..
        } => Notice::banner(
            Error,
            format!("device:{}", dir_str(*direction)),
            format!("notice.device.lost.{}", dir_str(*direction)),
        )
        .with_param("device", device),
        DeviceNotice::DeviceReconnected { direction, device } => Notice::banner(
            Info,
            format!("device:{}", dir_str(*direction)),
            format!("notice.device.reconnected.{}", dir_str(*direction)),
        )
        .with_param("device", device),
        DeviceNotice::BackendError { device, .. } => {
            Notice::toast(Warning, "notice.device.backend_error").with_param("device", device)
        }
        DeviceNotice::ResampleUnavailable {
            device,
            doc_rate_hz,
            device_rate_hz,
        } => Notice::toast(Error, "notice.device.resample_unavailable")
            .with_param("device", device)
            .with_param("doc_rate", doc_rate_hz.to_string())
            .with_param("rate", device_rate_hz.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use vox_engine::telemetry::vxtm_flags;
    use vox_engine::{HostId, TelemetryFrame};

    use super::*;

    #[test]
    fn device_prefs_round_trip_through_the_settings_dto() {
        let dto = DevicePrefsDto {
            host: "alsa".into(),
            input_device: Some("Mic".into()),
            input_channel: 2,
            output_device: Some("DAC".into()),
            sample_rate_hz: Some(44_100),
            buffer_size_frames: Some(256),
        };
        let prefs = DevicePrefs::from(&dto);
        assert_eq!(prefs.host, Some(HostId::Alsa));
        assert_eq!(prefs.buffer_size, BufferRequest::Frames(256));
        assert_eq!(prefs.input_channel, 2);
        assert_eq!(DevicePrefsDto::from(&prefs), dto);
        let auto = DevicePrefsDto {
            host: "asio".into(),
            buffer_size_frames: None,
            ..dto
        };
        let p = DevicePrefs::from(&auto);
        assert_eq!((p.host, p.buffer_size), (None, BufferRequest::Auto));
    }

    #[test]
    fn lost_and_reconnected_share_a_banner_id() {
        let lost = notice_from_device(&DeviceNotice::DeviceLost {
            direction: Direction::Output,
            device: "DAC".into(),
            recording_stopped: false,
            playback_stopped: true,
            recording_continues: false,
        });
        let back = notice_from_device(&DeviceNotice::DeviceReconnected {
            direction: Direction::Output,
            device: "DAC".into(),
        });
        assert!(lost.persistent && back.persistent);
        assert_eq!(lost.id, back.id);
        assert_eq!(lost.key, "notice.device.lost.output");
    }

    /// Writes the golden `VXTM` frame for the TS decoder test (ADR-003 §4: generated by the
    /// `export_bindings` run of `just gen-types`, checked by `just check-types`).
    #[test]
    fn export_bindings_vxtm_fixture() {
        let f = TelemetryFrame {
            seq: 7,
            flags: vxtm_flags::PLAYING | vxtm_flags::XRUN,
            playhead_sample: 123_456_789,
            playhead_time_ns: 987_654_321_000,
            rate: 44_100.0,
            out_peak_dbfs: -6.5,
            out_rms_dbfs: -20.25,
            in_peak_dbfs: f32::NEG_INFINITY,
            in_rms_dbfs: f32::NEG_INFINITY,
            audio_rev: 42,
            dropped_rt_events: 3,
        };
        let hex: String = f.encode().iter().map(|b| format!("{b:02x}")).collect();
        let ts = format!(
            "// Generated by the `export_bindings_vxtm_fixture` test (src-tauri, `just gen-types`). Do not edit.\n\
             \n\
             /** A `VXTM` frame encoded by Rust (`TelemetryFrame::encode`) for the TS decoder contract test. */\n\
             export const VXTM_FIXTURE_HEX =\n  \"{hex}\";\n\
             \n\
             /** The field values encoded in `VXTM_FIXTURE_HEX`. */\n\
             export const VXTM_FIXTURE_FIELDS = {{\n  \
             seq: 7,\n  flags: {flags},\n  playheadSample: 123456789,\n  playheadTimeNs: 987654321000,\n  \
             rate: 44100,\n  outPeakDbfs: -6.5,\n  outRmsDbfs: -20.25,\n  inPeakDbfs: -Infinity,\n  \
             inRmsDbfs: -Infinity,\n  audioRev: 42,\n  droppedRtEvents: 3,\n}} as const;\n",
            flags = f.flags,
        );
        let dir = std::env::var_os("TS_RS_EXPORT_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("bindings"));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("vxtm_fixture.ts"), ts).unwrap();
    }
}

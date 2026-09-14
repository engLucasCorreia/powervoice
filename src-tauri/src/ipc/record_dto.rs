//! Recording DTOs (S1-04): the record panel state (`record_state` event and `record_*` command
//! results). A new recording's `document_changed` payload is S1-03's `DocumentDto`.

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use vox_engine::record::{MonitorMode as EngineMonitorMode, RecordState};

use crate::ipc::DeviceStatusDto;
use crate::settings::MonitorMode;

/// The record panel state (SPEC-002 §2.1–§2.2, §2.7).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "bindings.ts")]
pub struct RecordStateDto {
    /// Configured input device (`None`: Arm and Record are unavailable).
    pub input_device: Option<String>,
    /// Applied 1-based input channel.
    pub input_channel: u32,
    pub input_status: DeviceStatusDto,
    /// Input armed (never persisted).
    pub armed: bool,
    /// The input stream is open (meter running).
    pub input_open: bool,
    /// The open input's rate = the rate of a new recording.
    pub input_rate_hz: Option<u32>,
    pub recording: bool,
    /// Stop was requested; the take is being finished and committed.
    pub finishing: bool,
    pub monitor: MonitorMode,
    /// Monitoring is audible now.
    pub monitoring: bool,
    /// T-107 (SPEC-002 §2.7, §4.4): the monitoring latency readout in µs (0.1 ms steps) — input
    /// latency + monitor buffer + rack latency (through-rack only) + output latency; `None` while
    /// the mode is Off or a stream is closed. Amber ≥ 20 ms, red ≥ 40 ms (UI).
    pub monitor_latency_us: Option<u32>,
    /// T-107: monitor dropouts (underruns faded out/in, overruns dropped back) since the output
    /// opened. The take is unaffected.
    pub monitor_dropouts: u32,
    /// H-10 item 4: dropout events so far this take (SPEC-002 §2.1's live amber counter), `0`
    /// while not recording.
    pub dropout_count: u32,
    /// H-11 (SPEC-002 §2.5, AC-13): remaining recording time on the session volume, in whole
    /// seconds; `None` when the free-space query failed.
    pub disk_remaining_s: Option<u64>,
}

impl From<&RecordState> for RecordStateDto {
    fn from(s: &RecordState) -> Self {
        Self {
            input_device: s.input_device.clone(),
            input_channel: u32::from(s.input_channel),
            input_status: s.input_status.into(),
            armed: s.armed,
            input_open: s.input_open,
            input_rate_hz: s.input_rate_hz,
            recording: s.recording,
            finishing: s.finishing,
            monitor: match s.monitor {
                EngineMonitorMode::Off => MonitorMode::Off,
                EngineMonitorMode::Dry => MonitorMode::Dry,
                EngineMonitorMode::ThroughRack => MonitorMode::ThroughRack,
            },
            monitoring: s.monitoring,
            monitor_latency_us: s.monitor_latency_us,
            monitor_dropouts: s.monitor_underruns.saturating_add(s.monitor_overruns),
            dropout_count: s.dropout_count,
            disk_remaining_s: s.disk_remaining_s,
        }
    }
}

/// Settings monitor mode → engine.
pub fn engine_monitor_mode(mode: MonitorMode) -> EngineMonitorMode {
    match mode {
        MonitorMode::Off => EngineMonitorMode::Off,
        MonitorMode::Dry => EngineMonitorMode::Dry,
        MonitorMode::ThroughRack => EngineMonitorMode::ThroughRack,
    }
}

#[cfg(test)]
mod tests {
    use vox_engine::device_state::DeviceStatus;

    use super::*;

    #[test]
    fn record_state_maps_to_the_dto() {
        let s = RecordState {
            input_device: Some("Mic".into()),
            input_channel: 2,
            input_status: DeviceStatus::Healthy,
            armed: true,
            input_open: true,
            input_rate_hz: Some(48_000),
            recording: true,
            finishing: false,
            monitor: EngineMonitorMode::ThroughRack,
            monitoring: true,
            monitor_latency_us: Some(23_700),
            monitor_underruns: 2,
            monitor_overruns: 1,
            dropout_count: 3,
            disk_remaining_s: Some(120),
        };
        let dto = RecordStateDto::from(&s);
        assert_eq!(dto.input_channel, 2);
        assert_eq!(dto.input_status, DeviceStatusDto::Healthy);
        assert_eq!(dto.monitor, MonitorMode::ThroughRack);
        assert_eq!(dto.monitor_latency_us, Some(23_700));
        assert_eq!(dto.monitor_dropouts, 3);
        assert_eq!(dto.dropout_count, 3);
        assert_eq!(dto.disk_remaining_s, Some(120));
        let json = serde_json::to_string(&dto).unwrap();
        assert!(json.contains("\"monitor\":\"through_rack\""), "{json}");
        for (mode, engine) in [
            (MonitorMode::Off, EngineMonitorMode::Off),
            (MonitorMode::Dry, EngineMonitorMode::Dry),
            (MonitorMode::ThroughRack, EngineMonitorMode::ThroughRack),
        ] {
            assert_eq!(engine_monitor_mode(mode), engine);
        }
    }
}

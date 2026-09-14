//! Recording commands (S1-04, SPEC-002 §2.1–§2.2, §2.7): arm, record start/stop, monitoring.
//! Thin: each forwards to the [`RecordingService`] (blocking work on `spawn_blocking`).
//!
//! `record_peaks_get` (H-07) is the odd one out: like `document_commands::peaks_get`, it encodes
//! the raw `VXPK` bytes here (ADR-003 §2, CLAUDE.md — bulk IPC data is binary, never JSON floats)
//! since that framing is not `RecordingService`'s business logic to own either.

use tauri::State;
use tauri::ipc::Response;
use vox_engine::record::{LIVE_PEAKS_SPB, LiveTakePeaks};
use vox_project::{VxpkHeader, encode_vxpk};

use crate::ipc::error::IpcError;
use crate::ipc::record_dto::RecordStateDto;
use crate::ipc::{Notice, NoticeLevel};
use crate::recording::RecordingService;
use crate::settings::{DefaultFormatDto, MonitorMode, Settings, SettingsStore};

/// ADR-003 §2's request cap, same as `document_commands::peaks_get`.
const MAX_LIVE_PEAKS_BUCKETS: u32 = 65_536;

async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, IpcError> + Send + 'static,
) -> Result<T, IpcError> {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| IpcError::internal(e.to_string()))?
}

/// The record panel state.
#[tauri::command]
pub async fn record_get(rec: State<'_, RecordingService>) -> Result<RecordStateDto, IpcError> {
    let rec = rec.inner().clone();
    blocking(move || rec.state()).await
}

/// Arms (opens the input, starts the meter) or disarms the input.
#[tauri::command]
pub async fn record_arm(
    rec: State<'_, RecordingService>,
    armed: bool,
) -> Result<RecordStateDto, IpcError> {
    let rec = rec.inner().clone();
    blocking(move || rec.arm(armed)).await
}

/// Starts a new recording. `replace`: the user confirmed replacing a document that has audio.
/// `format`: the New Recording dialog's chosen sample rate / bit depth (H-06); `None` uses the
/// current default format — Record with no document skips the dialog entirely (SPEC-002 §2.2).
#[tauri::command]
pub async fn record_start(
    rec: State<'_, RecordingService>,
    replace: bool,
    format: Option<DefaultFormatDto>,
) -> Result<RecordStateDto, IpcError> {
    let rec = rec.inner().clone();
    blocking(move || rec.start(replace, format)).await
}

/// Stops the recording; the take becomes the document when it is committed.
#[tauri::command]
pub async fn record_stop(rec: State<'_, RecordingService>) -> Result<RecordStateDto, IpcError> {
    let rec = rec.inner().clone();
    blocking(move || rec.stop()).await
}

/// `count` `(min, max)` buckets of the take being captured, from bucket `start_bucket`, as a
/// binary `VXPK` frame (H-07; `partial: true` always — the take is still growing). Zero buckets
/// at rate 0 while no take is being captured (the UI just keeps showing its last good frame).
#[tauri::command]
pub async fn record_peaks_get(
    rec: State<'_, RecordingService>,
    start_bucket: u32,
    count: u32,
) -> Result<Response, IpcError> {
    let rec = rec.inner().clone();
    let count = count.min(MAX_LIVE_PEAKS_BUCKETS);
    let peaks = blocking(move || Ok(rec.live_peaks(start_bucket, count))).await?;
    let peaks = peaks.unwrap_or(LiveTakePeaks {
        sample_rate_hz: 0,
        len_samples: 0,
        spb: LIVE_PEAKS_SPB,
        buckets: Vec::new(),
    });
    let header = VxpkHeader {
        request_id: 0,
        audio_rev: 0,
        start_sample: u64::from(start_bucket) * u64::from(peaks.spb),
        samples_per_bucket: peaks.spb,
        sample_rate_hz: peaks.sample_rate_hz,
        partial: true,
    };
    Ok(Response::new(encode_vxpk(&header, &peaks.buckets)))
}

/// Sets the monitoring mode and saves it as the preference (SPEC-002 §2.7). The first time
/// monitoring is enabled, a one-time "use headphones" hint is shown.
#[tauri::command]
pub async fn record_set_monitor(
    rec: State<'_, RecordingService>,
    settings: State<'_, SettingsStore>,
    mode: MonitorMode,
) -> Result<RecordStateDto, IpcError> {
    let service = rec.inner().clone();
    let state = blocking(move || service.set_monitor(mode)).await?;
    let before = settings.get();
    let mut saved = before.clone();
    let hint = apply_monitor_pref(&mut saved, mode);
    if saved != before {
        settings.set(saved)?;
    }
    if hint {
        rec.notify(Notice::toast(
            NoticeLevel::Info,
            "notice.monitor.headphones",
        ));
    }
    Ok(state)
}

/// T-304 (SPEC-022 §2.2, §4.9): Record with the current selection (`[start, end)` or `null`).
/// Resolves to a new recording (empty document), a punch-in over a non-empty selection, or
/// Insert/Overwrite at the selection start / cursor (a playing transport stops first).
#[tauri::command]
pub async fn record_start_at(
    rec: State<'_, RecordingService>,
    selection: Option<(u64, u64)>,
) -> Result<crate::ipc::RecordStartedDto, IpcError> {
    let rec = rec.inner().clone();
    blocking(move || rec.start_at(selection)).await
}

/// T-304 (SPEC-022 §2.13): the current device setup's recording offset (the readout).
#[tauri::command]
pub async fn record_offset_get(
    engine: State<'_, crate::audio::AudioEngine>,
    settings: State<'_, SettingsStore>,
) -> Result<crate::ipc::RecordOffsetDto, IpcError> {
    let engine = engine.handle().clone();
    let saved = settings.get();
    blocking(move || Ok(offset_readout(&engine, &saved.record_offsets))).await
}

/// T-304 (SPEC-022 §2.13, §2.14, AC-18): stores the offset for the current device setup
/// (calibration Apply, or a manual entry — the UI converts "N smp" at the device rate), clamped
/// to ±500 ms. Returns the new readout.
#[tauri::command]
pub async fn record_offset_set(
    engine: State<'_, crate::audio::AudioEngine>,
    settings: State<'_, SettingsStore>,
    offset_ms: f64,
    source: crate::settings::RecordOffsetSource,
    confidence: Option<f64>,
) -> Result<crate::ipc::RecordOffsetDto, IpcError> {
    let handle = engine.handle().clone();
    let setup = blocking(move || Ok(crate::recording::device_setup(&handle))).await?;
    let Some(setup) = setup else {
        return Err(IpcError::new(
            crate::ipc::IpcErrorCode::DeviceNotFound,
            "error.calibration.no_output",
        ));
    };
    let mut saved = settings.get();
    apply_offset(
        &mut saved,
        &setup,
        offset_ms,
        source,
        confidence,
        now_unix_ms(),
    );
    let saved = settings.set(saved)?;
    let engine = engine.handle().clone();
    blocking(move || Ok(offset_readout(&engine, &saved.record_offsets))).await
}

fn now_unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

/// Stores `offset_ms` (clamped to ±500 ms, SPEC-022 §3) under `setup`'s key.
fn apply_offset(
    saved: &mut Settings,
    setup: &crate::recording::DeviceSetup,
    offset_ms: f64,
    source: crate::settings::RecordOffsetSource,
    confidence: Option<f64>,
    now_ms: u64,
) {
    let offset_ms = if offset_ms.is_finite() {
        offset_ms.clamp(
            -vox_engine::record_op::MAX_OFFSET_MS,
            vox_engine::record_op::MAX_OFFSET_MS,
        )
    } else {
        0.0
    };
    crate::settings::upsert_record_offset(
        &mut saved.record_offsets,
        crate::settings::RecordOffsetEntry {
            host: setup.host.clone(),
            input_device: setup.input_device.clone(),
            output_device: setup.output_device.clone(),
            device_rate_hz: setup.device_rate_hz,
            offset_ms,
            source,
            updated_unix_ms: now_ms,
            confidence,
            buffer_frames: setup.buffer_frames,
        },
    );
}

/// The readout for the current device setup (SPEC-022 §2.13: "Not calibrated" = `source: null`).
fn offset_readout(
    engine: &vox_engine::EngineHandle,
    entries: &[crate::settings::RecordOffsetEntry],
) -> crate::ipc::RecordOffsetDto {
    let setup = crate::recording::device_setup(engine);
    let entry = setup
        .as_ref()
        .and_then(|s| crate::recording::record_offset_for(s, entries));
    crate::ipc::RecordOffsetDto {
        available: setup.is_some(),
        host: setup.as_ref().map(|s| s.host.clone()).unwrap_or_default(),
        input_device: setup
            .as_ref()
            .map(|s| s.input_device.clone())
            .unwrap_or_default(),
        output_device: setup
            .as_ref()
            .map(|s| s.output_device.clone())
            .unwrap_or_default(),
        device_rate_hz: setup.as_ref().map_or(0, |s| s.device_rate_hz),
        offset_ms: entry.map_or(0.0, |e| e.offset_ms),
        source: entry.map(|e| e.source),
        updated_unix_ms: entry.map(|e| e.updated_unix_ms),
        confidence: entry.and_then(|e| e.confidence),
        buffer_frames: entry.and_then(|e| e.buffer_frames),
        current_buffer_frames: setup.as_ref().and_then(|s| s.buffer_frames),
    }
}

/// T-107 (SPEC-002 §2.7, AC-12): saves `mode` as the preference; returns whether the one-time
/// headphone hint is due — the first time monitoring is enabled (remembered in the settings).
fn apply_monitor_pref(saved: &mut Settings, mode: MonitorMode) -> bool {
    saved.monitor_mode = mode;
    let hint = mode != MonitorMode::Off && !saved.monitor_hint_shown;
    saved.monitor_hint_shown |= hint;
    hint
}

#[cfg(test)]
mod tests {
    use super::*;

    /// AC-12: the feedback hint appears only the first time monitoring is enabled — not for Off,
    /// not again after it was shown (in any mode), and it survives as a saved setting.
    #[test]
    fn headphone_hint_is_shown_once() {
        let mut s = Settings::default();
        assert!(!apply_monitor_pref(&mut s, MonitorMode::Off));
        assert!(!s.monitor_hint_shown);
        assert!(apply_monitor_pref(&mut s, MonitorMode::Dry));
        assert!(s.monitor_hint_shown);
        assert_eq!(s.monitor_mode, MonitorMode::Dry);
        assert!(!apply_monitor_pref(&mut s, MonitorMode::Off));
        assert!(!apply_monitor_pref(&mut s, MonitorMode::ThroughRack));
        assert_eq!(s.monitor_mode, MonitorMode::ThroughRack);
    }
}

//! Audio commands (S1-01): devices, transport, telemetry channel, clock sync (ADR-003 §1, §3).
//! Thin: every command forwards to the engine handle and maps the result to a DTO.

use tauri::State;
use tauri::ipc::{Channel, InvokeResponseBody};
use vox_engine::device_state::DeviceStatus;
use vox_engine::{DevicePrefs, TelemetryFrame, TransportCommand};

use crate::audio::AudioEngine;
use crate::ipc::audio_dto::{DevicesDto, TransportStateDto};
use crate::ipc::error::IpcError;
use crate::settings::{DevicePrefsDto, SettingsStore};

fn engine_stopped() -> IpcError {
    IpcError::internal("the audio engine is not running")
}

fn transport(engine: &AudioEngine, cmd: TransportCommand) -> TransportStateDto {
    TransportStateDto::from(&engine.handle().transport(cmd))
}

/// The Audio Devices dialog's data (the engine's current device list; no new enumeration).
#[tauri::command]
pub async fn devices_list(engine: State<'_, AudioEngine>) -> Result<DevicesDto, IpcError> {
    let handle = engine.handle().clone();
    let view = tauri::async_runtime::spawn_blocking(move || handle.devices())
        .await
        .map_err(|e| IpcError::internal(e.to_string()))?
        .ok_or_else(engine_stopped)?;
    Ok(DevicesDto::from(&view))
}

/// Applies a device selection (output now; the input choice is stored for recording). The prefs
/// are saved once applied without the device being lost (SPEC-001 §2.5 "saved intent").
#[tauri::command]
pub async fn devices_select(
    engine: State<'_, AudioEngine>,
    settings: State<'_, SettingsStore>,
    prefs: DevicePrefsDto,
) -> Result<DevicesDto, IpcError> {
    let handle = engine.handle().clone();
    let engine_prefs = DevicePrefs::from(&prefs);
    let view = tauri::async_runtime::spawn_blocking(move || handle.select_devices(engine_prefs))
        .await
        .map_err(|e| IpcError::internal(e.to_string()))?
        .ok_or_else(engine_stopped)?;
    if view.output_status != DeviceStatus::Lost {
        let mut saved = settings.get();
        saved.device = prefs;
        settings.set(saved)?;
    }
    Ok(DevicesDto::from(&view))
}

/// Current transport state.
#[tauri::command]
pub async fn transport_get(engine: State<'_, AudioEngine>) -> Result<TransportStateDto, IpcError> {
    Ok(TransportStateDto::from(&engine.handle().transport_state()))
}

/// Play from the playhead.
#[tauri::command]
pub async fn transport_play(engine: State<'_, AudioEngine>) -> Result<TransportStateDto, IpcError> {
    Ok(transport(&engine, TransportCommand::Play))
}

/// Pause (keeps the heard position).
#[tauri::command]
pub async fn transport_pause(
    engine: State<'_, AudioEngine>,
) -> Result<TransportStateDto, IpcError> {
    Ok(transport(&engine, TransportCommand::Pause))
}

/// Stop (returns to the play-start position).
#[tauri::command]
pub async fn transport_stop(engine: State<'_, AudioEngine>) -> Result<TransportStateDto, IpcError> {
    Ok(transport(&engine, TransportCommand::Stop))
}

/// Play from the selection start, or 0.
#[tauri::command]
pub async fn transport_play_from_start(
    engine: State<'_, AudioEngine>,
) -> Result<TransportStateDto, IpcError> {
    Ok(transport(&engine, TransportCommand::PlayFromStart))
}

/// Seek to 0 (keeps playing).
#[tauri::command]
pub async fn transport_return_to_start(
    engine: State<'_, AudioEngine>,
) -> Result<TransportStateDto, IpcError> {
    Ok(transport(&engine, TransportCommand::ReturnToStart))
}

/// Move the playhead to a document position.
#[tauri::command]
pub async fn transport_seek(
    engine: State<'_, AudioEngine>,
    position_samples: u64,
) -> Result<TransportStateDto, IpcError> {
    Ok(transport(&engine, TransportCommand::Seek(position_samples)))
}

/// Streams binary `VXTM` frames (playhead anchor + output meter) at 60 Hz over `channel`,
/// replacing any previous subscriber.
#[tauri::command]
pub async fn telemetry_subscribe(
    engine: State<'_, AudioEngine>,
    channel: Channel,
) -> Result<(), IpcError> {
    engine
        .handle()
        .set_telemetry_sink(Some(Box::new(move |frame: &TelemetryFrame| {
            let _ = channel.send(InvokeResponseBody::Raw(frame.encode().to_vec()));
        })));
    Ok(())
}

/// The engine's app clock (ns since the process epoch), for the UI's clock sync (ADR-003 §3).
#[tauri::command]
pub async fn clock_now_ns() -> Result<u64, IpcError> {
    Ok(vox_engine::backend::app_now_ns())
}

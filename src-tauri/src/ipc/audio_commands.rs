//! Audio commands (S1-01): devices, transport, telemetry channel, clock sync (ADR-003 §1, §3).
//! Thin: every command forwards to the engine handle and maps the result to a DTO.

use tauri::State;
use tauri::ipc::{Channel, InvokeResponseBody};
use vox_engine::device_state::DeviceStatus;
use vox_engine::{DevicePrefs, EngineHandle, TelemetryFrame, TransportCommand};

use crate::audio::AudioEngine;
use crate::ipc::audio_dto::{DevicesDto, TransportStateDto};
use crate::ipc::error::IpcError;
use crate::settings::{DevicePrefsDto, SettingsStore};

fn engine_stopped() -> IpcError {
    IpcError::internal("the audio engine is not running")
}

/// Runs `f` on a blocking thread: engine calls wait for the control thread's answer.
async fn blocking<T: Send + 'static>(
    engine: &AudioEngine,
    f: impl FnOnce(&EngineHandle) -> T + Send + 'static,
) -> Result<T, IpcError> {
    let handle = engine.handle().clone();
    tauri::async_runtime::spawn_blocking(move || f(&handle))
        .await
        .map_err(|e| IpcError::internal(e.to_string()))
}

async fn transport(
    engine: &AudioEngine,
    cmd: TransportCommand,
) -> Result<TransportStateDto, IpcError> {
    let state = blocking(engine, move |h| h.transport(cmd)).await?;
    Ok(TransportStateDto::from(&state))
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
    let state = blocking(&engine, |h| h.transport_state()).await?;
    Ok(TransportStateDto::from(&state))
}

/// Play from the playhead.
#[tauri::command]
pub async fn transport_play(engine: State<'_, AudioEngine>) -> Result<TransportStateDto, IpcError> {
    transport(&engine, TransportCommand::Play).await
}

/// Pause (keeps the heard position).
#[tauri::command]
pub async fn transport_pause(
    engine: State<'_, AudioEngine>,
) -> Result<TransportStateDto, IpcError> {
    transport(&engine, TransportCommand::Pause).await
}

/// Stop (returns to the play-start position).
#[tauri::command]
pub async fn transport_stop(engine: State<'_, AudioEngine>) -> Result<TransportStateDto, IpcError> {
    transport(&engine, TransportCommand::Stop).await
}

/// Play from the selection start, or 0.
#[tauri::command]
pub async fn transport_play_from_start(
    engine: State<'_, AudioEngine>,
) -> Result<TransportStateDto, IpcError> {
    transport(&engine, TransportCommand::PlayFromStart).await
}

/// Seek to 0 (keeps playing).
#[tauri::command]
pub async fn transport_return_to_start(
    engine: State<'_, AudioEngine>,
) -> Result<TransportStateDto, IpcError> {
    transport(&engine, TransportCommand::ReturnToStart).await
}

/// Move the playhead to a document position.
#[tauri::command]
pub async fn transport_seek(
    engine: State<'_, AudioEngine>,
    position_samples: u64,
) -> Result<TransportStateDto, IpcError> {
    transport(&engine, TransportCommand::Seek(position_samples)).await
}

/// H-37 (SPEC-003 §2.1): the Loop toggle. Off during looped playback lets the pass finish.
#[tauri::command]
pub async fn transport_set_loop(
    engine: State<'_, AudioEngine>,
    enabled: bool,
) -> Result<TransportStateDto, IpcError> {
    let state = blocking(&engine, move |h| h.set_loop(enabled)).await?;
    Ok(TransportStateDto::from(&state))
}

/// H-37: the UI's time selection `[start, end)` (`null`: none) — Play from start's position
/// and, while loop is on, the loop region.
#[tauri::command]
pub async fn transport_set_selection(
    engine: State<'_, AudioEngine>,
    selection: Option<(u64, u64)>,
) -> Result<TransportStateDto, IpcError> {
    let state = blocking(&engine, move |h| h.set_selection(selection)).await?;
    Ok(TransportStateDto::from(&state))
}

/// Streams binary `VXTM` frames (playhead anchor + output meter) at 60 Hz over `channel`,
/// replacing any previous subscriber.
#[tauri::command]
pub async fn telemetry_subscribe(
    engine: State<'_, AudioEngine>,
    channel: Channel,
) -> Result<(), IpcError> {
    blocking(&engine, move |h| {
        h.set_telemetry_sink(Some(Box::new(move |frame: &TelemetryFrame| {
            let _ = channel.send(InvokeResponseBody::Raw(frame.encode().to_vec()));
        })));
    })
    .await
}

/// The engine's app clock (ns since the process epoch), for the UI's clock sync (ADR-003 §3).
#[tauri::command]
pub async fn clock_now_ns() -> Result<u64, IpcError> {
    Ok(vox_engine::backend::app_now_ns())
}

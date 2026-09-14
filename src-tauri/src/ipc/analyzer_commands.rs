//! Live output analyzer commands (T-208, SPEC-007 §2.9/§4.9): subscribe/set-response/unsubscribe
//! over a binary `VXSA` channel, mirroring `module_telemetry_subscribe` (H-03) except that each
//! subscriber gets its own channel from the one shared computation (ADR-003), so subscribing
//! returns an id used to target `analyzer_set_response`/`analyzer_unsubscribe`.

use tauri::State;
use tauri::ipc::{Channel, InvokeResponseBody};
use vox_engine::AnalyzerFrame;

use crate::audio::AudioEngine;
use crate::ipc::analyzer_dto::AnalyzerResponseDto;
use crate::ipc::error::IpcError;

/// Subscribes to the live output analyzer at `response`; returns the subscriber id. Frames flow
/// only while at least one subscriber exists (`ANALYZER_ON`).
#[tauri::command]
pub async fn analyzer_subscribe(
    engine: State<'_, AudioEngine>,
    channel: Channel,
    response: AnalyzerResponseDto,
) -> Result<u32, IpcError> {
    let handle = engine.handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        handle.analyzer_subscribe(
            Box::new(move |frame: &AnalyzerFrame| {
                let _ = channel.send(InvokeResponseBody::Raw(frame.encode()));
            }),
            response.into(),
        )
    })
    .await
    .map_err(|e| IpcError::internal(e.to_string()))
}

/// Changes a subscriber's averaging response.
#[tauri::command]
pub async fn analyzer_set_response(
    engine: State<'_, AudioEngine>,
    id: u32,
    response: AnalyzerResponseDto,
) -> Result<(), IpcError> {
    let handle = engine.handle().clone();
    tauri::async_runtime::spawn_blocking(move || handle.analyzer_set_response(id, response.into()))
        .await
        .map_err(|e| IpcError::internal(e.to_string()))
}

/// Removes a subscriber; the tap turns off once none remain.
#[tauri::command]
pub async fn analyzer_unsubscribe(engine: State<'_, AudioEngine>, id: u32) -> Result<(), IpcError> {
    let handle = engine.handle().clone();
    tauri::async_runtime::spawn_blocking(move || handle.analyzer_unsubscribe(id))
        .await
        .map_err(|e| IpcError::internal(e.to_string()))
}

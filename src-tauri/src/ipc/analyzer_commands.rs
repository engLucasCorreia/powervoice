//! Live output analyzer commands (T-208, SPEC-007 §2.9/§4.9): subscribe/set-response/unsubscribe
//! over a binary `VXSA` channel, mirroring `module_telemetry_subscribe` (H-03) except that each
//! subscriber gets its own channel from the one shared computation (ADR-003), so subscribing
//! returns an id used to target `analyzer_set_response`/`analyzer_unsubscribe`.

use tauri::State;
use tauri::ipc::{Channel, InvokeResponseBody};
use vox_dsp::diagnostics::VoiceReport;
use vox_engine::{AnalyzerFrame, InspectorFrame};

use crate::audio::AudioEngine;
use crate::ipc::analyzer_dto::{AnalyzerResponseDto, InspectorConfigDto, VoiceReportDto};
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

/// H-42 (SPEC-007 §8.8): subscribes to live voice diagnostics — a JSON [`VoiceReportDto`] on
/// `channel` about 10 times a second, only when it changed. Returns the id to pass to
/// `analyzer_unsubscribe`.
#[tauri::command]
pub async fn analyzer_voice_subscribe(
    engine: State<'_, AudioEngine>,
    channel: Channel<VoiceReportDto>,
) -> Result<u32, IpcError> {
    let handle = engine.handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        handle.analyzer_voice_subscribe(Box::new(move |report: &VoiceReport| {
            let _ = channel.send(VoiceReportDto::from(report));
        }))
    })
    .await
    .map_err(|e| IpcError::internal(e.to_string()))
}

/// H-42 (SPEC-007 §8.3/§8.9): subscribes a Spectrum Inspector stream — binary `VXIS` frames on
/// `channel` at `config`'s FFT size, window and response. Returns the id to pass to
/// `analyzer_inspector_configure`/`analyzer_unsubscribe`.
#[tauri::command]
pub async fn analyzer_inspector_subscribe(
    engine: State<'_, AudioEngine>,
    channel: Channel,
    config: InspectorConfigDto,
) -> Result<u32, IpcError> {
    let handle = engine.handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        handle.analyzer_inspector_subscribe(
            Box::new(move |frame: &InspectorFrame| {
                let _ = channel.send(InvokeResponseBody::Raw(frame.encode()));
            }),
            config.into(),
        )
    })
    .await
    .map_err(|e| IpcError::internal(e.to_string()))
}

/// H-42: changes an Inspector stream's FFT size, window or response.
#[tauri::command]
pub async fn analyzer_inspector_configure(
    engine: State<'_, AudioEngine>,
    id: u32,
    config: InspectorConfigDto,
) -> Result<(), IpcError> {
    let handle = engine.handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        handle.analyzer_inspector_configure(id, config.into())
    })
    .await
    .map_err(|e| IpcError::internal(e.to_string()))
}

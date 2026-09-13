//! Rack commands (S3-01, SPEC-012 §2.1–§2.6). Thin: every command forwards a `vox_engine`
//! `RackCommand` and maps the result to a DTO (ADR-003); Rust — not this layer, not the UI —
//! parses parameter text and formats it back (`param_set_text`, SPEC-012 §2.6).

use tauri::State;
use vox_engine::RackCommand;
use vox_rack::ParamId;

use crate::audio::AudioEngine;
use crate::ipc::error::IpcError;
use crate::ipc::rack_dto::{ModuleDescriptorDto, RackStateDto, rack_ipc_error};

/// Runs `cmd` on the engine's control thread and maps the result to a [`RackStateDto`].
async fn apply(engine: &AudioEngine, cmd: RackCommand) -> Result<RackStateDto, IpcError> {
    let handle = engine.handle().clone();
    let result = tauri::async_runtime::spawn_blocking(move || handle.rack_command(cmd))
        .await
        .map_err(|e| IpcError::internal(e.to_string()))?;
    Ok(RackStateDto::from(&result.map_err(rack_ipc_error)?))
}

/// The registered modules (the Add-module menu, grouped client-side by `features`).
#[tauri::command]
pub async fn rack_list_modules(
    engine: State<'_, AudioEngine>,
) -> Result<Vec<ModuleDescriptorDto>, IpcError> {
    let handle = engine.handle().clone();
    let descriptors = tauri::async_runtime::spawn_blocking(move || handle.rack_registry())
        .await
        .map_err(|e| IpcError::internal(e.to_string()))?;
    Ok(descriptors.iter().map(Into::into).collect())
}

/// The current rack state (initial load; also after any reconnect).
#[tauri::command]
pub async fn rack_get(engine: State<'_, AudioEngine>) -> Result<RackStateDto, IpcError> {
    let handle = engine.handle().clone();
    let snapshot = tauri::async_runtime::spawn_blocking(move || handle.rack_snapshot())
        .await
        .map_err(|e| IpcError::internal(e.to_string()))?;
    Ok(RackStateDto::from(&snapshot))
}

/// Adds a registered module at `index` (`0..=len`), live (SPEC-012 §2.1–§2.2).
#[tauri::command]
pub async fn rack_add(
    engine: State<'_, AudioEngine>,
    module_id: String,
    index: usize,
) -> Result<RackStateDto, IpcError> {
    apply(&engine, RackCommand::Add { module_id, index }).await
}

/// Removes the slot at `slot` (index), live.
#[tauri::command]
pub async fn rack_remove(
    engine: State<'_, AudioEngine>,
    slot: usize,
) -> Result<RackStateDto, IpcError> {
    apply(&engine, RackCommand::Remove { index: slot }).await
}

/// Moves the slot at `from` to `to` (drag-reorder), live.
#[tauri::command]
pub async fn rack_move(
    engine: State<'_, AudioEngine>,
    from: usize,
    to: usize,
) -> Result<RackStateDto, IpcError> {
    apply(&engine, RackCommand::Move { from, to }).await
}

/// Per-slot bypass toggle (15 ms crossfade, SPEC-012 §2.3).
#[tauri::command]
pub async fn rack_bypass(
    engine: State<'_, AudioEngine>,
    slot: usize,
    on: bool,
) -> Result<RackStateDto, IpcError> {
    apply(&engine, RackCommand::SetBypass { index: slot, on }).await
}

/// Whole-rack A/B (listening only, SPEC-012 §2.3).
#[tauri::command]
pub async fn rack_ab(engine: State<'_, AudioEngine>, on: bool) -> Result<RackStateDto, IpcError> {
    apply(&engine, RackCommand::SetAb { on }).await
}

/// Restarts a slot's instance from its committed state (Restart of a failed slot, ADR-005 §12).
#[tauri::command]
pub async fn rack_restart(
    engine: State<'_, AudioEngine>,
    slot: usize,
) -> Result<RackStateDto, IpcError> {
    apply(&engine, RackCommand::Restart { index: slot }).await
}

/// Sets a parameter from a normalized `[0, 1]` slider position (SPEC-012 §2.4, §2.6). The UI is
/// responsible for sending at most one call per animation frame while dragging.
#[tauri::command]
pub async fn param_set_normalized(
    engine: State<'_, AudioEngine>,
    slot: usize,
    id: u32,
    value: f64,
) -> Result<RackStateDto, IpcError> {
    apply(
        &engine,
        RackCommand::SetParamNormalized {
            index: slot,
            id: ParamId(id),
            value,
        },
    )
    .await
}

/// Sets a parameter from typed text; Rust parses it (SPEC-012 §2.6). Unparseable text is
/// rejected (`error.rack_rejected`) without changing anything.
#[tauri::command]
pub async fn param_set_text(
    engine: State<'_, AudioEngine>,
    slot: usize,
    id: u32,
    text: String,
) -> Result<RackStateDto, IpcError> {
    apply(
        &engine,
        RackCommand::SetParamText {
            index: slot,
            id: ParamId(id),
            text,
        },
    )
    .await
}

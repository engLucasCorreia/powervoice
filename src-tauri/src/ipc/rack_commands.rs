//! Rack commands (S3-01, SPEC-012 §2.1–§2.6). Thin: every command forwards a `vox_engine`
//! `RackCommand` and maps the result to a DTO (ADR-003); Rust — not this layer, not the UI —
//! parses parameter text and formats it back (`param_set_text`, SPEC-012 §2.6).

use tauri::State;
use tauri::ipc::{Channel, InvokeResponseBody};
use vox_engine::{ModuleTelemetryFrame, RackCommand};
use vox_rack::{ModuleDescriptor, ParamId};

use crate::audio::AudioEngine;
use crate::ipc::error::IpcError;
use crate::ipc::rack_dto::{ModuleDescriptorDto, RackStateDto, ResponseCurveDto, rack_ipc_error};
use crate::settings::SettingsStore;

/// Runs `cmd` on the engine's control thread and maps the result to a [`RackStateDto`]. `pub(crate)`
/// so `preset_commands` (T-406) reuses it for `ApplyModulePreset`/`ResetToDefault`.
pub(crate) async fn apply(
    engine: &AudioEngine,
    cmd: RackCommand,
) -> Result<RackStateDto, IpcError> {
    let handle = engine.handle().clone();
    let result = tauri::async_runtime::spawn_blocking(move || handle.rack_command(cmd))
        .await
        .map_err(|e| IpcError::internal(e.to_string()))?;
    Ok(RackStateDto::from(&result.map_err(rack_ipc_error)?))
}

/// The registered modules (the Add-module menu, grouped client-side by `features`). T-804 item
/// 5: a module id in `Settings.plugins.disabled` is hidden here — it's still registered (an
/// existing document that already uses it still loads it) and still listed by `plugins_list`,
/// just not offered for a *new* insertion.
#[tauri::command]
pub async fn rack_list_modules(
    engine: State<'_, AudioEngine>,
    settings: State<'_, SettingsStore>,
) -> Result<Vec<ModuleDescriptorDto>, IpcError> {
    let handle = engine.handle().clone();
    let descriptors = tauri::async_runtime::spawn_blocking(move || handle.rack_registry())
        .await
        .map_err(|e| IpcError::internal(e.to_string()))?;
    let disabled = settings.get().plugins.disabled;
    Ok(visible_modules(&descriptors, &disabled))
}

/// [`rack_list_modules`]'s filter, split out so it's unit-testable without a running engine
/// (same pattern as `commands::telemetry_rate_change`).
fn visible_modules(
    descriptors: &[ModuleDescriptor],
    disabled: &[String],
) -> Vec<ModuleDescriptorDto> {
    descriptors
        .iter()
        .filter(|d| !disabled.iter().any(|id| id == &d.id))
        .map(Into::into)
        .collect()
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

/// Sets a parameter from a plain value (S3-07, SPEC-015 §2.6.6): the EQ graph's draggable nodes
/// send Hz/dB/Q values directly — the inverse of the display axis mapping, not taper code, so
/// this still isn't the UI running filter math. Same clamp-quantize/mirror/echo path as
/// [`param_set_normalized`]/[`param_set_text`]. The UI is responsible for sending at most one
/// call per animation frame per parameter while dragging.
#[tauri::command]
pub async fn param_set_plain(
    engine: State<'_, AudioEngine>,
    slot: usize,
    id: u32,
    value: f64,
) -> Result<RackStateDto, IpcError> {
    apply(
        &engine,
        RackCommand::SetParamPlain {
            index: slot,
            id: ParamId(id),
            value,
        },
    )
    .await
}

/// The EQ graph's response curve at `points` (S3-07, SPEC-015 §2.6.6, lean slice: JSON of
/// ≤ `vox_engine::MAX_RESPONSE_CURVE_POINTS` frequencies — the binary `VXRC` frame is
/// hardening). `points` are Hz values the UI's log-frequency axis computed (not filter math);
/// Rust evaluates the target slot's `ResponseCurve` extension at them, on the control thread,
/// from the parameter mirror's current values. An oversized `points` list is truncated, not
/// rejected.
#[tauri::command]
pub async fn rack_response_curve(
    engine: State<'_, AudioEngine>,
    slot: usize,
    points: Vec<f64>,
) -> Result<ResponseCurveDto, IpcError> {
    let handle = engine.handle().clone();
    let result = tauri::async_runtime::spawn_blocking(move || handle.response_curve(slot, points))
        .await
        .map_err(|e| IpcError::internal(e.to_string()))?;
    Ok(result.map_err(rack_ipc_error)?.into())
}

/// Streams binary `VXMT` module-telemetry frames (H-03, SPEC-016 §4.12: every slot's
/// `Telemetry` values, e.g. the true-peak limiter's gain reduction) at the telemetry rate over
/// `channel`, replacing any previous subscriber. Frames flow while at least one slot has the
/// extension; channel descriptions are in `RackSlotDto::telemetry`.
#[tauri::command]
pub async fn module_telemetry_subscribe(
    engine: State<'_, AudioEngine>,
    channel: Channel,
) -> Result<(), IpcError> {
    let handle = engine.handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        handle.set_module_telemetry_sink(Some(Box::new(move |frame: &ModuleTelemetryFrame| {
            let _ = channel.send(InvokeResponseBody::Raw(frame.encode()));
        })));
    })
    .await
    .map_err(|e| IpcError::internal(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use vox_module_api::{LocalizedText, MODULE_API_VERSION, Version};

    fn descriptor(id: &str) -> ModuleDescriptor {
        ModuleDescriptor {
            id: id.to_string(),
            version: Version::new(1, 0, 0),
            name: LocalizedText::plain(id),
            vendor: "V".into(),
            description: LocalizedText::plain(""),
            url: None,
            features: vec!["audio-effect".into()],
            state_format_version: 1,
            api_version: MODULE_API_VERSION,
        }
    }

    /// T-804 item 5's acceptance criterion: "a disabled plugin is hidden from the registry list".
    #[test]
    fn a_disabled_plugin_is_hidden_from_the_add_module_list() {
        let descriptors = [
            descriptor("org.powervoice.gain"),
            descriptor("clap:com.acme.deesser"),
        ];
        let visible = visible_modules(&descriptors, &["clap:com.acme.deesser".to_string()]);
        assert_eq!(visible.len(), 1);
        assert_eq!(visible[0].id, "org.powervoice.gain");
    }

    #[test]
    fn nothing_disabled_shows_everything() {
        let descriptors = [descriptor("org.powervoice.gain")];
        assert_eq!(visible_modules(&descriptors, &[]).len(), 1);
    }
}

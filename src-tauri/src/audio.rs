//! Audio engine wiring (S1-01): starts the engine with the saved device prefs and the built-in
//! modules, and forwards its events to the UI (`transport_state`, `devices_changed`, `notice`).

use std::sync::Arc;

use tauri::{AppHandle, Emitter, Runtime};
use vox_engine::backend::cpal::CpalBackend;
use vox_engine::{Engine, EngineConfig, EngineEvent, EngineHandle};
use vox_rack::Registry;

use crate::ipc::{
    DevicesDto, EventName, RecordStateDto, TransportStateDto, emit_notice, notice_from_device,
};
use crate::settings::DevicePrefsDto;

/// The running engine, managed as Tauri state.
pub struct AudioEngine {
    handle: EngineHandle,
    _engine: Engine,
}

impl AudioEngine {
    /// The engine's synchronous API.
    pub fn handle(&self) -> &EngineHandle {
        &self.handle
    }
}

/// Starts the engine on the real (cpal) backend. Devices are enumerated and the output opened
/// on the engine's own threads; the UI hears about it through `devices_changed`.
pub fn start<R: Runtime>(
    app: &AppHandle<R>,
    prefs: &DevicePrefsDto,
) -> anyhow::Result<AudioEngine> {
    let registry = Arc::new(Registry::with_factories(vox_modules::builtin_factories())?);
    let mut config = EngineConfig::new(Arc::new(CpalBackend::new()), registry);
    config.prefs = prefs.into();
    let app = app.clone();
    config.events = Arc::new(move |event| forward(&app, event));
    let engine = Engine::start(config)?;
    Ok(AudioEngine {
        handle: engine.handle(),
        _engine: engine,
    })
}

/// Runs on the engine's control thread: map to DTOs and emit (never call back into the engine).
fn forward<R: Runtime>(app: &AppHandle<R>, event: EngineEvent) {
    let result = match event {
        EngineEvent::Transport(state) => app.emit(
            EventName::transport_state.as_str(),
            TransportStateDto::from(&state),
        ),
        EngineEvent::Devices(view) => {
            tracing::info!(
                host = ?view.host,
                devices = view.snapshot.devices.len(),
                output = ?view.output_device,
                rate_hz = ?view.output_rate_hz,
                status = ?view.output_status,
                "audio devices"
            );
            app.emit(EventName::devices_changed.as_str(), DevicesDto::from(&view))
        }
        EngineEvent::Notice(notice) => {
            tracing::info!(?notice, "audio device notice");
            emit_notice(app, notice_from_device(&notice))
        }
        EngineEvent::Record(state) => app.emit(
            EventName::record_state.as_str(),
            RecordStateDto::from(&state),
        ),
    };
    if let Err(e) = result {
        tracing::warn!(error = %e, "emitting an engine event failed");
    }
}

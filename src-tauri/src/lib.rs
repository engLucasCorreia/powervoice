//! `powervoice-app`: the thin Tauri command/event layer (ADR-001 §3, ADR-003).
//!
//! No business logic lives here. This crate wires Tauri commands to domain crates and maps
//! domain types to the DTOs in [`ipc`].

pub mod audio;
pub mod document;
pub mod export;
pub mod ipc;
pub mod logging;
pub mod recording;
pub mod settings;
#[cfg(feature = "spike")]
pub mod spike;
pub mod webkit;

use tauri::Manager as _;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Keep the guard alive for the whole run: dropping it stops the non-blocking log writer
    // from flushing (T-104).
    let _log_guard = logging::init();
    tracing::info!(version = env!("CARGO_PKG_VERSION"), "PowerVoice starting");

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(settings::SettingsStore::load_default())
        .setup(|app| {
            // S1-01: the audio engine starts with the saved device prefs (SPEC-001 §2.5).
            let settings = app.state::<settings::SettingsStore>().get();
            let engine = audio::start(app.handle(), &settings.device)?;
            // S1-03: the document service shares the engine handle (`set_document` after
            // open/save-as) and owns the one open session under the OS data dir.
            let documents = document::DocumentService::new(
                document::default_sessions_dir(),
                engine.handle().clone(),
            );
            // S1-04: recording into the document service (default format, saved monitoring).
            let recording = recording::start(
                app.handle(),
                engine.handle().clone(),
                documents.clone(),
                &settings,
            );
            // S4-04: the export job service (its own registry instance — modules are stateless
            // per-instance, so sharing the engine's would only save one small allocation).
            let export = export::start(app.handle().clone(), documents.clone())?;
            app.manage(documents);
            app.manage(recording);
            app.manage(engine);
            app.manage(export);
            Ok(())
        })
        .invoke_handler(ipc::invoke_handler())
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

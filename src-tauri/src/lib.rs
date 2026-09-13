//! `powervoice-app`: the thin Tauri command/event layer (ADR-001 §3, ADR-003).
//!
//! No business logic lives here. This crate wires Tauri commands to domain crates and maps
//! domain types to the DTOs in [`ipc`].

pub mod audio;
pub mod document;
pub mod ipc;
pub mod logging;
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
            let prefs = app.state::<settings::SettingsStore>().get().device;
            let engine = audio::start(app.handle(), &prefs)?;
            // S1-03: the document service shares the engine handle (`set_document` after
            // open/save-as) and owns the one open session under the OS data dir.
            app.manage(document::DocumentService::new(
                document::default_sessions_dir(),
                engine.handle().clone(),
            ));
            app.manage(engine);
            Ok(())
        })
        .invoke_handler(ipc::invoke_handler())
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

//! `powervoice-app`: the thin Tauri command/event layer (ADR-001 §3, ADR-003).
//!
//! No business logic lives here. This crate wires Tauri commands to domain crates and maps
//! domain types to the DTOs in [`ipc`].

pub mod ipc;
pub mod logging;
pub mod settings;
#[cfg(feature = "spike")]
pub mod spike;
pub mod webkit;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Keep the guard alive for the whole run: dropping it stops the non-blocking log writer
    // from flushing (T-104).
    let _log_guard = logging::init();
    tracing::info!(version = env!("CARGO_PKG_VERSION"), "PowerVoice starting");

    tauri::Builder::default()
        .manage(settings::SettingsStore::load_default())
        .invoke_handler(ipc::invoke_handler())
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

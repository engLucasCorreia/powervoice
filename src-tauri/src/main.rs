// Prevents an additional console window on Windows in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // ADR-009 Amendment 1 / MEMORY D-016: must run before any threads start (the Tauri builder
    // in `run()` spawns some) and before the WebView is created. See `webkit::apply_dmabuf_default`
    // for the `SAFETY` justification.
    powervoice_app_lib::webkit::apply_dmabuf_default();
    powervoice_app_lib::run();
}

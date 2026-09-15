//! `powervoice-app`: the thin Tauri command/event layer (ADR-001 §3, ADR-003).
//!
//! No business logic lives here. This crate wires Tauri commands to domain crates and maps
//! domain types to the DTOs in [`ipc`].

pub mod audio;
pub mod bake;
pub mod calibration;
pub mod document;
pub mod export;
pub mod housekeeping;
pub mod ipc;
pub mod logging;
pub mod loudness;
pub mod normalize;
pub mod nr_capture;
pub mod plugins;
pub mod presets;
pub mod recording;
pub mod settings;
#[cfg(feature = "spike")]
pub mod spike;
#[cfg(test)]
mod test_util;
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
        .manage(presets::PresetStores::load_default())
        .setup(|app| {
            // S1-01: the audio engine starts with the saved device prefs (SPEC-001 §2.5).
            let settings = app.state::<settings::SettingsStore>().get();
            let sessions_dir = document::default_sessions_dir();
            // T-804 (ADR-008 §6, item 1): the instant, sandbox-free cached-plugins load, before
            // any composition root builds its registry — start-up never blocks on a sandbox.
            plugins::configure(&settings.plugins.custom_folders);
            // H-11 (SPEC-002 §2.5): the engine polls this volume's free space for the recording
            // disk floor and the record panel's remaining-time display.
            let engine = audio::start(app.handle(), &settings.device, &sessions_dir)?;
            // H-16 (SPEC-003 §3): the saved telemetry rate applies from the start, not just after
            // the first `settings_set` (which only pushes it live on a *change*).
            engine
                .handle()
                .set_telemetry_rate_hz(settings.telemetry_rate_hz);
            // S1-03: the document service shares the engine handle (`set_document` after
            // open/save-as) and owns the one open session under the OS data dir.
            let documents = document::DocumentService::new(sessions_dir, engine.handle().clone());
            // T-301 (SPEC-004 §2.4): the saved "Memory for audio" budget.
            documents.set_memory_budget_mib(settings.memory_budget_mib);
            // T-301 (SPEC-004 §2.8, ADR-004 §10): finish interrupted deletions and remove
            // cleanly closed sessions before anything opens; recoverable ones wait for the UI's
            // recovery dialog (`recovery_list`).
            let recoverable = documents.recovery_list().len();
            tracing::info!(recoverable, "session start-up cleanup done");
            // S1-04: recording into the document service (default format, saved monitoring).
            let recording = recording::start(
                app.handle(),
                engine.handle().clone(),
                documents.clone(),
                &settings,
            );
            // T-204: the spectrogram tile service (its own worker threads + content-keyed tile
            // cache; ADR-001 §4, SPEC-007 §4.1). Shared as an `Arc` so commands can hand it to
            // `spawn_blocking`. Built before export/bake/documents' own wiring below (H-30) so
            // each can hold `begin_background_job()` for its own job/save.
            let spectro = std::sync::Arc::new(vox_engine::spectro::SpectroService::new(
                vox_engine::spectro::SpectroConfig::default(),
            ));
            // H-30 (SPEC-007 §4.1): "save" is the third job the spec lists alongside export/bake.
            documents.set_spectro(std::sync::Arc::clone(&spectro));
            // S4-04/H-08: the export job service (its own registry instance — modules are
            // stateless per-instance, so sharing the engine's would only save one small
            // allocation) renders the live rack via `EngineHandle::rack_model()`.
            let export = export::start(
                app.handle().clone(),
                documents.clone(),
                engine.handle().clone(),
                std::sync::Arc::clone(&spectro),
            )?;
            // S3-06: the Capture Noise Print job service.
            let nr_capture = nr_capture::start(
                app.handle().clone(),
                engine.handle().clone(),
                documents.clone(),
            )?;
            // S4-01: the loudness analysis job service ("processed" mode reads the live rack via
            // `EngineHandle::rack_model()`).
            let loudness = loudness::start(
                app.handle().clone(),
                documents.clone(),
                engine.handle().clone(),
            )?;
            // H-09: the normalize job service (peak + LUFS) — no engine handle needed, unlike
            // export/nr_capture/loudness (SPEC-010 never touches the rack).
            let normalize = normalize::start(app.handle().clone(), documents.clone())?;
            // T-602: the Bake rack job service (renders the live rack like export; halves the
            // spectrogram's tile workers while it runs, SPEC-007 §4.1).
            let bake = bake::start(
                app.handle().clone(),
                documents.clone(),
                engine.handle().clone(),
                std::sync::Arc::clone(&spectro),
            )?;
            // T-304 (SPEC-022 §2.14): the loopback latency calibration job.
            let calibration = calibration::start(app.handle(), engine.handle().clone());
            app.manage(spectro);
            app.manage(documents);
            app.manage(recording);
            app.manage(calibration);
            app.manage(engine);
            app.manage(export);
            app.manage(nr_capture);
            app.manage(loudness);
            app.manage(normalize);
            app.manage(bake);
            // T-301: rack/view state journaling (2 s) and the disk budget check (10 s / after
            // every edit, SPEC-004 §2.5).
            let housekeeping = housekeeping::start(
                app.handle().clone(),
                app.state::<document::DocumentService>().inner().clone(),
            );
            app.manage(housekeeping);
            // T-804 (ADR-008 §6, item 1): the authoritative background rescan — every composition
            // root above has already built (and registered) its own registry, so any plugin this
            // finds hot-adds into all of them, not just the engine's.
            let progress_app = app.handle().clone();
            let done_app = app.handle().clone();
            plugins::start_background_scan(
                move |done, total, path| {
                    if let Err(e) = ipc::emit_plugin_scan_progress(&progress_app, done, total, path)
                    {
                        tracing::warn!(error = %e, "plugin_scan_progress emit failed");
                    }
                },
                move |summary| {
                    tracing::info!(
                        scanned = summary.scanned,
                        cached = summary.cached,
                        indexed = summary.indexed,
                        failed = summary.failed,
                        blocklisted_now = summary.blocklisted_now,
                        blocklisted = summary.blocklisted,
                        effects = summary.effects,
                        newly_registered = summary.newly_registered.len(),
                        "plugin scan finished"
                    );
                    if let Err(e) = ipc::emit_plugin_scan_summary(&done_app, summary) {
                        tracing::warn!(error = %e, "plugin_scan_progress summary emit failed");
                    }
                },
            );
            Ok(())
        })
        .invoke_handler(ipc::invoke_handler())
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

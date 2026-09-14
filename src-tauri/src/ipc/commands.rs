use tauri::State;

use crate::audio::AudioEngine;
use crate::ipc::dto::AppInfo;
use crate::ipc::error::IpcError;
use crate::settings::{Settings, SettingsStore};

/// Returns basic app identity info (name + version). Tauri command handlers are always `async`
/// so sync commands still run off the main thread (CLAUDE.md / ADR-003).
#[tauri::command]
pub async fn app_info() -> Result<AppInfo, IpcError> {
    Ok(AppInfo {
        name: "PowerVoice".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
    })
}

/// Returns the current settings (T-104). Served from the in-memory `SettingsStore` cache — this
/// command never touches disk.
#[tauri::command]
pub async fn settings_get(store: State<'_, SettingsStore>) -> Result<Settings, IpcError> {
    Ok(store.get())
}

/// Replaces the settings file (atomic write: temp file + rename) and returns the canonical saved
/// value. Per SPEC-001 §2.5's "saved intent" rule, this command stores exactly what it's given —
/// it has no knowledge of runtime fallbacks (unsupported rate, clamped buffer size), so a caller
/// that only wants to persist the user's *chosen* value, not whatever got applied, is doing the
/// right thing by calling this with that chosen value.
///
/// H-16: `telemetry_rate_hz` is also a *live* setting (unlike most others here, which only take
/// effect on the next document/stream) — when it changes, this pushes it to the running engine
/// right away (`EngineHandle::set_telemetry_rate_hz`, cheap: a control-thread cadence change,
/// never touches the audio thread).
#[tauri::command]
pub async fn settings_set(
    store: State<'_, SettingsStore>,
    documents: State<'_, crate::document::DocumentService>,
    engine: State<'_, AudioEngine>,
    settings: Settings,
) -> Result<Settings, IpcError> {
    let before = store.get();
    let saved = store.set(settings)?;
    // T-301 (SPEC-004 §2.4): "Memory for audio" applies without a restart.
    documents.set_memory_budget_mib(saved.memory_budget_mib);
    // H-16: the telemetry rate applies live too (only when it actually changed).
    if let Some(rate_hz) = telemetry_rate_change(&before, &saved) {
        engine.handle().set_telemetry_rate_hz(rate_hz);
    }
    Ok(saved)
}

/// `Some(new rate)` when `next.telemetry_rate_hz` differs from `before`'s — split out from
/// `settings_set` so the "did it actually change" logic is unit-testable without a Tauri app /
/// running engine (same pattern as `record_commands::apply_monitor_pref`).
fn telemetry_rate_change(before: &Settings, next: &Settings) -> Option<u32> {
    (next.telemetry_rate_hz != before.telemetry_rate_hz).then_some(next.telemetry_rate_hz)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn telemetry_rate_change_is_none_when_unchanged() {
        let before = Settings::default();
        let next = Settings::default();
        assert_eq!(telemetry_rate_change(&before, &next), None);
    }

    #[test]
    fn telemetry_rate_change_reports_the_new_rate() {
        let before = Settings::default();
        let next = Settings {
            telemetry_rate_hz: 30,
            ..Settings::default()
        };
        assert_eq!(telemetry_rate_change(&before, &next), Some(30));
    }

    #[test]
    fn telemetry_rate_change_ignores_unrelated_setting_changes() {
        let before = Settings::default();
        let next = Settings {
            monitor_hint_shown: true,
            ..Settings::default()
        };
        assert_eq!(telemetry_rate_change(&before, &next), None);
    }
}

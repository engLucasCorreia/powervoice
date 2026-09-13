//! Events emitted to the UI (ADR-003). Mirrors the command registry (`macros.rs`, `mod.rs`).

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

crate::ipc_events!(
    notice,
    transport_state,
    devices_changed,
    record_state,
    document_changed,
    history_state,
    clipboard_changed
);

/// A user-facing notice (ADR-003 `notice` event). Two shapes, distinguished by `persistent`:
/// - a **toast** (`persistent: false`, `id: None`): transient, auto-dismisses in the UI (e.g.
///   SPEC-001 §2.3's fallback/reconnect notices).
/// - a **banner** (`persistent: true`, `id: Some(..)`): stays until the user dismisses it or
///   another `Notice` with the same `id` replaces/clears it (e.g. SPEC-001 §2.3's device-lost
///   banner turning into a brief "reconnected" notice).
///
/// `key`/`params` is an i18n message key plus its placeholder values (CLAUDE.md: every
/// user-facing string goes through i18n) — this event never carries pre-rendered text.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "bindings.ts")]
pub struct Notice {
    pub level: NoticeLevel,
    pub key: String,
    pub params: HashMap<String, String>,
    pub persistent: bool,
    /// Stable id for a banner that a later `Notice` can replace or clear. `None` for one-shot
    /// toasts, which are never replaced (each is its own event).
    pub id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "bindings.ts", rename_all = "snake_case")]
pub enum NoticeLevel {
    Info,
    Warning,
    Error,
}

impl Notice {
    pub fn toast(level: NoticeLevel, key: impl Into<String>) -> Self {
        Self {
            level,
            key: key.into(),
            params: HashMap::new(),
            persistent: false,
            id: None,
        }
    }

    pub fn banner(level: NoticeLevel, id: impl Into<String>, key: impl Into<String>) -> Self {
        Self {
            level,
            key: key.into(),
            params: HashMap::new(),
            persistent: true,
            id: Some(id.into()),
        }
    }

    #[must_use]
    pub fn with_param(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.params.insert(name.into(), value.into());
        self
    }
}

/// Emits a `notice` event (ADR-003). Generic over `R: tauri::Runtime`, like every Tauri handle
/// this crate holds onto (MEMORY.md T-007 gotcha: `AppHandle<R>` must stay generic to work
/// inside the rest of the IPC machinery). Intended for later tickets (device loss, recording,
/// disk pressure, ...) that need to push a notice from wherever they detect the condition.
pub fn emit_notice<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    notice: Notice,
) -> tauri::Result<()> {
    use tauri::Emitter as _;
    app.emit(EventName::notice.as_str(), notice)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Guards `EVENT_NAMES` against drifting apart from the generated `EventName` TS union
    /// (mirrors the `command_names_match_command_name_variants` test in `mod.rs`).
    #[test]
    fn event_names_match_event_name_variants() {
        let serialized: Vec<String> = EVENT_NAME_VARIANTS
            .iter()
            .map(|v| {
                serde_json::to_string(v)
                    .unwrap()
                    .trim_matches('"')
                    .to_string()
            })
            .collect();
        let expected: Vec<String> = EVENT_NAMES.iter().map(|s| s.to_string()).collect();
        assert_eq!(serialized, expected);
    }

    #[test]
    fn event_name_as_str_matches_the_literal_name() {
        assert_eq!(EventName::notice.as_str(), "notice");
    }

    #[test]
    fn toast_is_transient_and_unidentified() {
        let toast = Notice::toast(NoticeLevel::Info, "notice.example").with_param("n", "3");
        assert!(!toast.persistent);
        assert_eq!(toast.id, None);
        assert_eq!(toast.params.get("n").map(String::as_str), Some("3"));
        assert_eq!(toast.key, "notice.example");
    }

    #[test]
    fn banner_is_persistent_and_identified() {
        let banner = Notice::banner(NoticeLevel::Error, "device:output", "notice.device_lost");
        assert!(banner.persistent);
        assert_eq!(banner.id.as_deref(), Some("device:output"));
        assert_eq!(banner.level, NoticeLevel::Error);
    }
}

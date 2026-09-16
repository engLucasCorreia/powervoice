//! Events emitted to the UI (ADR-003). Mirrors the command registry (`macros.rs`, `mod.rs`).

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

crate::ipc_events!(
    notice,
    transport_state,
    devices_changed,
    rack_changed,
    param_changed,
    rack_latency,
    record_state,
    document_changed,
    history_state,
    clipboard_changed,
    job_progress,
    loudness_report,
    normalize_result,
    recent_files_changed,
    record_phase,
    record_finished,
    calibration_result,
    import_started,
    plugin_scan_progress,
    spectrum_report
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
///
/// H-17: `cleared` (additive, defaults to `false` so older payloads still parse) instead
/// *removes* the banner sharing `id` — for a condition that resolves itself with nothing left for
/// the user to act on (SPEC-004 §2.5's disk-almost-full banner once space is reclaimed), unlike
/// the device-lost→reconnected pattern above, which replaces it with an acknowledgeable one.
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
    /// `true`: remove the banner sharing `id` instead of showing anything (`level`/`key` are
    /// ignored). Only meaningful with `id: Some(..)`.
    #[serde(default)]
    pub cleared: bool,
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
            cleared: false,
        }
    }

    pub fn banner(level: NoticeLevel, id: impl Into<String>, key: impl Into<String>) -> Self {
        Self {
            level,
            key: key.into(),
            params: HashMap::new(),
            persistent: true,
            id: Some(id.into()),
            cleared: false,
        }
    }

    /// H-17: removes the banner `id` (e.g. SPEC-004 §2.5's disk-almost-full banner once space is
    /// reclaimed) instead of replacing it with anything.
    pub fn clear_banner(id: impl Into<String>) -> Self {
        Self {
            level: NoticeLevel::Info,
            key: String::new(),
            params: HashMap::new(),
            persistent: true,
            id: Some(id.into()),
            cleared: true,
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

/// A long-running, cancellable job's kind (ADR-003 `job_progress`; S4-04 is the first job). New
/// job kinds add a variant here rather than a new event, so the frontend has one progress/cancel
/// pattern for every job.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "bindings.ts", rename_all = "snake_case")]
pub enum JobKind {
    Export,
    /// T-209: opening (importing) a file (SPEC-005 §2.3) — `document_open`'s job, cancelled by
    /// `document_open_cancel`. Its finished document arrives separately as `document_changed`
    /// (the `document_open` command's own return value already gives it to the caller that
    /// started the job; other listeners, e.g. an import progress dialog, don't need it repeated
    /// here).
    Import,
    /// S3-06: Capture Noise Print (SPEC-014 §2.3).
    NrCapture,
    /// S4-01: the loudness analysis job (`loudness_analyze_start`) — its finished report arrives
    /// separately as a `loudness_report` event (`job_progress`'s `fraction`/`state` alone can't
    /// carry it).
    LoudnessAnalyze,
    /// H-09: peak normalize (`edit_normalize_peak_start`, SPEC-010) — its finished edit result
    /// arrives separately as a `normalize_result` event.
    NormalizePeak,
    /// H-09: LUFS normalize (`edit_normalize_lufs_start`) — mirrors `NormalizePeak`.
    NormalizeLufs,
    /// T-304 (SPEC-022 §2.14): a loopback latency calibration (`calibration_run`) — its result
    /// arrives separately as a `calibration_result` event.
    Calibration,
    /// T-602: Bake rack (`edit_bake_start`) — renders the live rack into the selection or the
    /// whole file; a successful commit arrives as `document_changed`/`history_state` plus a
    /// `notice.bake.done` toast.
    Bake,
    /// H-42 (SPEC-007 §8.5): the long-term average spectrum job (`spectrum_analyze_start`) —
    /// its diagnostics arrive as a `spectrum_report` event, its curves via
    /// `spectrum_analyze_curve`.
    SpectrumAnalyze,
    /// H-56 (SPEC-008 §2.6.1): a cross-document paste that needs the clipboard's materialized
    /// audio imported (and, on a rate mismatch, resampled) into the open document — `edit_paste`'s
    /// own job, cancelled by `edit_paste_cancel`. Its finished edit result is `edit_paste`'s own
    /// return value (mirrors `Import`), not a separate event.
    Paste,
}

/// `job_progress`'s lifecycle. `Running` fractions are monotonically non-decreasing in `[0, 1]`;
/// exactly one of `Done`/`Cancelled`/`Failed` follows the last `Running` event for a `job_id`
/// (SPEC-010 §2.8's normalize-job convention, generalized here for S4-04's export job).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "bindings.ts", rename_all = "snake_case")]
pub enum JobState {
    Running,
    Done,
    Cancelled,
    Failed,
}

/// `job_progress` event payload (ADR-003; ≤ 10 Hz per job). `job_id` distinguishes overlapping or
/// stale jobs of the same `kind`; a failure's message goes out separately as a `notice` (the same
/// convention `error.*`/`notice.*` i18n keys use everywhere else), not inline here.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "bindings.ts")]
pub struct JobProgressDto {
    pub job_id: u32,
    pub kind: JobKind,
    pub state: JobState,
    pub fraction: f32,
}

/// Emits a `job_progress` event (ADR-003).
pub fn emit_job_progress<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    progress: JobProgressDto,
) -> tauri::Result<()> {
    use tauri::Emitter as _;
    app.emit(EventName::job_progress.as_str(), progress)
}

/// H-20 (SPEC-005 §2.3): the import job's document shell — file name, rate and (when the container
/// states one) length — emitted once, right after the probe and before the decode loop starts.
/// The frontend uses it to show "the document shell" ("‹name›" in the title, an estimated ruler
/// length) immediately, well before `document_changed` arrives at commit.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "bindings.ts")]
pub struct ImportStartedDto {
    pub job_id: u32,
    pub name: String,
    pub sample_rate_hz: u32,
    /// `None` when the container doesn't state a sample count (SPEC-005 §2.3: "estimated for
    /// formats without a sample count" — the UI shows an indeterminate shell then).
    pub len_samples: Option<u64>,
}

/// Emits an `import_started` event (H-20).
pub fn emit_import_started<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    info: ImportStartedDto,
) -> tauri::Result<()> {
    use tauri::Emitter as _;
    app.emit(EventName::import_started.as_str(), info)
}

/// Emits a `loudness_report` event (S4-01): the loudness analysis job's finished report, once its
/// `job_progress` reports `JobState::Done`.
pub fn emit_loudness_report<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    report: crate::ipc::loudness_dto::LoudnessReportDto,
) -> tauri::Result<()> {
    use tauri::Emitter as _;
    app.emit(EventName::loudness_report.as_str(), report)
}

/// Emits a `spectrum_report` event (H-42): a finished long-term average job's diagnostics.
pub fn emit_spectrum_report<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    report: crate::ipc::spectrum_dto::SpectrumReportDto,
) -> tauri::Result<()> {
    use tauri::Emitter as _;
    app.emit(EventName::spectrum_report.as_str(), report)
}

/// Emits a `normalize_result` event (H-09): a finished normalize job's edit result, once its
/// `job_progress` reports `JobState::Done`.
pub fn emit_normalize_result<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    result: crate::ipc::normalize_dto::NormalizeResultDto,
) -> tauri::Result<()> {
    use tauri::Emitter as _;
    app.emit(EventName::normalize_result.as_str(), result)
}

/// The background plugin scan's summary (T-804 item 1), once per rescan — the `plugin_scan_progress`
/// event's final message (`current_path: None`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "bindings.ts")]
pub struct PluginScanSummaryDto {
    pub scanned: u32,
    pub cached: u32,
    pub failed: u32,
    pub blocklisted_now: u32,
    pub blocklisted: u32,
    pub effects: u32,
    /// Module ids newly registered by this scan (T-804 item 1: "insertable without a restart").
    pub newly_registered: Vec<String>,
}

impl From<vox_plugin_host::catalog::ScanSummary> for PluginScanSummaryDto {
    fn from(s: vox_plugin_host::catalog::ScanSummary) -> Self {
        Self {
            scanned: s.scanned as u32,
            cached: s.cached as u32,
            failed: s.failed as u32,
            blocklisted_now: s.blocklisted_now as u32,
            blocklisted: s.blocklisted as u32,
            effects: s.effects as u32,
            newly_registered: s.newly_registered,
        }
    }
}

/// `plugin_scan_progress` event payload (T-804 item 1): a tick while scanning (`current_path:
/// Some(..)`, `summary: None`), or the final one (`current_path: None`, `summary: Some(..)`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "bindings.ts")]
pub struct PluginScanProgressDto {
    pub done: u32,
    pub total: u32,
    pub current_path: Option<String>,
    pub summary: Option<PluginScanSummaryDto>,
}

/// Emits a `plugin_scan_progress` tick (T-804 item 1).
pub fn emit_plugin_scan_progress<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    done: usize,
    total: usize,
    current_path: &std::path::Path,
) -> tauri::Result<()> {
    use tauri::Emitter as _;
    app.emit(
        EventName::plugin_scan_progress.as_str(),
        PluginScanProgressDto {
            done: done as u32,
            total: total as u32,
            current_path: Some(current_path.to_string_lossy().into_owned()),
            summary: None,
        },
    )
}

/// Emits `plugin_scan_progress`'s final summary (T-804 item 1).
pub fn emit_plugin_scan_summary<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    summary: vox_plugin_host::catalog::ScanSummary,
) -> tauri::Result<()> {
    use tauri::Emitter as _;
    let summary = PluginScanSummaryDto::from(summary);
    app.emit(
        EventName::plugin_scan_progress.as_str(),
        PluginScanProgressDto {
            done: summary.scanned + summary.cached,
            total: summary.scanned + summary.cached,
            current_path: None,
            summary: Some(summary),
        },
    )
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

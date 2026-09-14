//! Session housekeeping timer (T-301): glue only — the decisions live in `vox_project`
//! (`Session::housekeep`, `budget::decide`) and `DocumentService`.
//!
//! One background thread wakes every 2 s (ADR-004 §6 `state_debounce_s`) to journal the rack/view
//! `state` when it changed, and runs SPEC-004 §2.5's disk check every 10 s
//! (`disk_check_interval_s`) or right after a committed edit ([`kick`]). It turns the outcome
//! into notices: "the N oldest undo steps were removed (freed X)" and the persistent "Disk
//! almost full — save your work." banner. Nothing runs while recording (the service skips it).

use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

use tauri::{AppHandle, Emitter, Manager, Runtime};
use vox_project::SystemFreeSpace;

use crate::document::DocumentService;
use crate::ipc::{DocumentDto, EventName, HistoryStateDto, Notice, NoticeLevel, emit_notice};

/// ADR-004 §6: rack/view state is journaled at most this long after it changed.
const STATE_DEBOUNCE: Duration = Duration::from_secs(2);
/// SPEC-004 §3 `disk_check_interval_s`.
const DISK_CHECK_INTERVAL: Duration = Duration::from_secs(10);
/// The banner id the disk-almost-full warning replaces itself under.
const DISK_FULL_BANNER: &str = "disk_almost_full";

/// Managed as Tauri state; [`kick`] asks for a disk check now.
pub struct Housekeeping {
    kick: Sender<()>,
}

/// Starts the thread (it ends when the app drops the managed [`Housekeeping`]).
pub fn start<R: Runtime>(app: AppHandle<R>, documents: DocumentService) -> Housekeeping {
    let (kick, rx) = mpsc::channel();
    let spawned = std::thread::Builder::new()
        .name("housekeeping".into())
        .spawn(move || run(&app, &documents, &rx));
    if let Err(error) = spawned {
        tracing::error!(%error, "starting the housekeeping thread failed");
    }
    Housekeeping { kick }
}

/// Requests a disk check soon (SPEC-004 §2.5: "after every committed edit").
pub fn kick<R: Runtime>(app: &AppHandle<R>) {
    if let Some(h) = app.try_state::<Housekeeping>() {
        let _ = h.kick.send(());
    }
}

fn run<R: Runtime>(app: &AppHandle<R>, documents: &DocumentService, rx: &mpsc::Receiver<()>) {
    let free = SystemFreeSpace;
    let mut last_disk_check = Instant::now();
    let mut warned_full = false;
    loop {
        let kicked = match rx.recv_timeout(STATE_DEBOUNCE) {
            Ok(()) => true,
            Err(RecvTimeoutError::Timeout) => false,
            Err(RecvTimeoutError::Disconnected) => return,
        };
        while rx.try_recv().is_ok() {}
        documents.journal_state_if_changed();
        if !kicked && last_disk_check.elapsed() < DISK_CHECK_INTERVAL {
            continue;
        }
        last_disk_check = Instant::now();
        let Some(report) = documents.housekeeping(&free) else {
            continue;
        };
        if report.dropped_undo > 0 {
            let notice = Notice::toast(NoticeLevel::Warning, "notice.disk.undo_dropped")
                .with_param("count", report.dropped_undo.to_string())
                .with_param("size", format_bytes(report.freed_bytes));
            emit(app, |a| emit_notice(a, notice));
            let history: HistoryStateDto = documents.history_state().into();
            emit(app, |a| a.emit(EventName::history_state.as_str(), history));
            let info: DocumentDto = documents.info().into();
            emit(app, |a| a.emit(EventName::document_changed.as_str(), info));
        }
        if report.almost_full && !warned_full {
            let notice = Notice::banner(
                NoticeLevel::Warning,
                DISK_FULL_BANNER,
                "notice.disk.almost_full",
            );
            emit(app, |a| emit_notice(a, notice));
        }
        warned_full = report.almost_full;
    }
}

fn emit<R: Runtime>(app: &AppHandle<R>, f: impl FnOnce(&AppHandle<R>) -> tauri::Result<()>) {
    if let Err(error) = f(app) {
        tracing::warn!(%error, "emitting a housekeeping event failed");
    }
}

/// "4.1 GB" / "512 MB" (decimal units, one decimal) for notices.
pub fn format_bytes(bytes: u64) -> String {
    const GB: f64 = 1e9;
    const MB: f64 = 1e6;
    let b = bytes as f64;
    if b >= GB {
        format!("{:.1} GB", b / GB)
    } else {
        format!("{:.0} MB", (b / MB).max(1.0))
    }
}

#[cfg(test)]
mod tests {
    use super::format_bytes;

    #[test]
    fn bytes_read_like_the_spec_notice() {
        assert_eq!(format_bytes(4_100_000_000), "4.1 GB");
        assert_eq!(format_bytes(67_108_864), "67 MB");
        assert_eq!(format_bytes(10), "1 MB");
    }
}

//! Recent files commands (T-306, SPEC-018 §2.12). Thin: state lives in
//! [`crate::settings::SettingsStore`]; this module only maps to/from the DTO, resolves existence
//! within a shared wall-clock budget when the File menu opens, and fires `recent_files_changed`.

use std::path::Path;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use tauri::{AppHandle, Emitter, Runtime, State};

use crate::ipc::error::IpcError;
use crate::ipc::events::EventName;
use crate::ipc::recent_files_dto::RecentFileDto;
use crate::settings::{self, SettingsStore};

/// SPEC-018 `recent_check_budget_ms` (§2.12, §3): existence checks share this wall-clock budget
/// when the File menu opens. Network paths must not freeze the menu past it.
const EXISTENCE_CHECK_BUDGET_MS: u64 = 300;

/// Runs `exists(path)` for every entry on its own thread — never the async command's own task,
/// and never the UI thread — and waits for all of them, but never longer than `budget` in total.
/// An entry whose check hasn't reported by the deadline (a stalled `stat` on an unreachable
/// network mount, H-15/AC-16) gets `None`; the others keep their real result regardless of order,
/// so one stalled entry never delays or hides the rest.
fn checked_existence(
    paths: &[String],
    budget: Duration,
    exists: impl Fn(&Path) -> bool + Send + Sync + Clone + 'static,
) -> Vec<Option<bool>> {
    let (tx, rx) = mpsc::channel();
    for (index, path) in paths.iter().cloned().enumerate() {
        let tx = tx.clone();
        let exists = exists.clone();
        thread::spawn(move || {
            let result = exists(Path::new(&path));
            // The receiver may already be gone (deadline passed and `rx` dropped) — that's fine,
            // this thread just finishes on its own past the budget.
            let _ = tx.send((index, result));
        });
    }
    drop(tx);

    let mut results = vec![None; paths.len()];
    let mut remaining = results.len();
    let deadline = Instant::now() + budget;
    while remaining > 0 {
        let now = Instant::now();
        if now >= deadline {
            break;
        }
        match rx.recv_timeout(deadline - now) {
            Ok((index, result)) => {
                results[index] = Some(result);
                remaining -= 1;
            }
            Err(_) => break,
        }
    }
    results
}

fn to_dtos_with_budget(
    entries: &[settings::RecentFileEntry],
    budget: Duration,
    exists: impl Fn(&Path) -> bool + Send + Sync + Clone + 'static,
) -> Vec<RecentFileDto> {
    let paths: Vec<String> = entries.iter().map(|e| e.path.clone()).collect();
    let resolved = checked_existence(&paths, budget, exists);
    entries
        .iter()
        .zip(resolved)
        .map(|(entry, exists)| RecentFileDto::from_entry(entry, exists))
        .collect()
}

fn to_dtos(entries: &[settings::RecentFileEntry]) -> Vec<RecentFileDto> {
    entries.iter().map(RecentFileDto::from).collect()
}

fn emit_recent_files_changed<R: Runtime>(
    app: &AppHandle<R>,
    entries: &[settings::RecentFileEntry],
) {
    if let Err(error) = app.emit(EventName::recent_files_changed.as_str(), to_dtos(entries)) {
        tracing::warn!(%error, "emitting recent_files_changed failed");
    }
}

/// The current list, most-recent-first (SPEC-018 §2.12), with `exists` resolved within
/// [`EXISTENCE_CHECK_BUDGET_MS`] (the File menu calls this every time it opens).
#[tauri::command]
pub async fn recent_files_get(
    settings: State<'_, SettingsStore>,
) -> Result<Vec<RecentFileDto>, IpcError> {
    let entries = settings.get().recent_files;
    Ok(to_dtos_with_budget(
        &entries,
        Duration::from_millis(EXISTENCE_CHECK_BUDGET_MS),
        |p| p.exists(),
    ))
}

/// Removes one entry (the File menu's "Remove" action on a missing file, or "Remove from List").
#[tauri::command]
pub async fn recent_files_remove<R: Runtime>(
    app: AppHandle<R>,
    settings: State<'_, SettingsStore>,
    path: String,
) -> Result<Vec<RecentFileDto>, IpcError> {
    let mut current = settings.get();
    settings::remove_recent_file(&mut current.recent_files, &path);
    let saved = settings.set(current)?;
    emit_recent_files_changed(&app, &saved.recent_files);
    Ok(to_dtos(&saved.recent_files))
}

/// Empties the list (SPEC-018 §2.12: "deletes no user data" — no confirmation needed).
#[tauri::command]
pub async fn recent_files_clear<R: Runtime>(
    app: AppHandle<R>,
    settings: State<'_, SettingsStore>,
) -> Result<(), IpcError> {
    let mut current = settings.get();
    current.recent_files.clear();
    let saved = settings.set(current)?;
    emit_recent_files_changed(&app, &saved.recent_files);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entries(paths: &[&str]) -> Vec<settings::RecentFileEntry> {
        paths
            .iter()
            .map(|p| settings::RecentFileEntry {
                path: (*p).to_string(),
                opened_at: "2026-09-13T08:41:07Z".to_string(),
            })
            .collect()
    }

    /// SPEC-018 §2.12/AC-16: a stalled `stat` (an unreachable network mount) must not hold up the
    /// others or the overall check past the budget.
    #[test]
    fn a_stalled_entry_never_delays_the_others_and_the_whole_check_stays_within_budget() {
        let paths = vec![
            "/vo/fast-exists".to_string(),
            "/vo/stalled".to_string(),
            "/vo/fast-missing".to_string(),
        ];
        let budget = Duration::from_millis(EXISTENCE_CHECK_BUDGET_MS);
        let start = Instant::now();
        let results = checked_existence(&paths, budget, |p| {
            if p == Path::new("/vo/stalled") {
                thread::sleep(Duration::from_secs(2));
                true
            } else {
                p == Path::new("/vo/fast-exists")
            }
        });
        let elapsed = start.elapsed();
        assert!(
            elapsed < budget + Duration::from_millis(150),
            "took {elapsed:?}, budget was {budget:?}"
        );
        assert_eq!(results[0], Some(true));
        assert_eq!(results[1], None, "stalled past the budget -> unresolved");
        assert_eq!(results[2], Some(false));
    }

    /// The common case: every check finishes well inside the budget, in order.
    #[test]
    fn every_entry_resolves_when_none_of_them_stall() {
        let paths = vec!["/vo/a".to_string(), "/vo/b".to_string()];
        let results = checked_existence(
            &paths,
            Duration::from_millis(EXISTENCE_CHECK_BUDGET_MS),
            |p| p == Path::new("/vo/a"),
        );
        assert_eq!(results, vec![Some(true), Some(false)]);
    }

    /// `to_dtos_with_budget` carries the resolved (or unresolved) existence straight into the DTO.
    #[test]
    fn to_dtos_with_budget_carries_existence_into_the_dto() {
        let list = entries(&["/vo/a", "/vo/gone"]);
        let dtos = to_dtos_with_budget(
            &list,
            Duration::from_millis(EXISTENCE_CHECK_BUDGET_MS),
            |p| p == Path::new("/vo/a"),
        );
        assert_eq!(dtos[0].exists, Some(true));
        assert_eq!(dtos[1].exists, Some(false));
    }
}

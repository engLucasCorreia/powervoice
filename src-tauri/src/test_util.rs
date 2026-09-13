//! Test-only helpers shared by the service tests.

use std::path::{Path, PathBuf};

/// A fresh directory for one test, under this test process's `powervoice-test-<pid>` parent in
/// the system temp dir. Store-backed tests preallocate 64 MB+ segments and nothing removes them
/// when the test process exits, so the first call sweeps the parents of finished runs (and the
/// older flat `powervoice-app-*-<pid>-<nanos>` dirs) — without it they filled the tmpfs `/tmp`
/// until unrelated tests failed with "Disk quota exceeded".
pub(crate) fn tmp_dir(tag: &str) -> PathBuf {
    static SWEEP: std::sync::Once = std::sync::Once::new();
    SWEEP.call_once(sweep_finished_runs);
    let dir = std::env::temp_dir()
        .join(format!("powervoice-test-{}", std::process::id()))
        .join(format!(
            "{tag}-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Removes test dirs whose test process is gone: `powervoice-test-<pid>` parents,
/// `powervoice-app-<kind>-<pid>` parents and flat `powervoice-app-<…>-<pid>-<nanos>` dirs. Uses
/// `/proc`, so it is a no-op where that doesn't exist.
fn sweep_finished_runs() {
    if !Path::new("/proc/self").exists() {
        return;
    }
    let Ok(entries) = std::fs::read_dir(std::env::temp_dir()) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        let pid = if let Some(rest) = name.strip_prefix("powervoice-test-") {
            rest
        } else if name.starts_with("powervoice-app-") {
            let fields: Vec<&str> = name.split('-').collect();
            match fields.len() {
                4 => fields[3],
                n if n >= 5 => fields[n - 2],
                _ => continue,
            }
        } else {
            continue;
        };
        let Ok(pid) = pid.parse::<u32>() else {
            continue;
        };
        if pid != std::process::id() && !Path::new(&format!("/proc/{pid}")).exists() {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

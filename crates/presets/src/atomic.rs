//! Atomic JSON preset writes (T-406), the same temp-file + rename pattern as
//! `src-tauri::settings::save` and `vox_project::sidecar` (MEMORY.md T-306).

use std::io::Write as _;
use std::path::{Path, PathBuf};

use crate::error::PresetError;

/// Writes `json` to `path` atomically: a hidden temp file in the same directory, `fsync`, then
/// `rename` over the target (atomic on the same filesystem on every platform this project ships
/// for). A process that crashes between the write and the rename leaves only the temp file
/// behind — [`list_json_names`] ignores it, so a half-written save is never mistaken for a
/// preset.
pub fn write_atomic(path: &Path, json: &[u8]) -> Result<(), PresetError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp_path = temp_path(path);
    {
        let mut file = std::fs::File::create(&tmp_path)?;
        file.write_all(json)?;
        file.sync_all()?;
    }
    std::fs::rename(&tmp_path, path)?;
    Ok(())
}

/// The temp path [`write_atomic`] writes before renaming over `path`: hidden (leading `.`), so a
/// directory listing never shows it as a preset, and tagged with this process id so two
/// processes racing to save don't collide.
fn temp_path(path: &Path) -> PathBuf {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("preset.json");
    path.with_file_name(format!(".{name}.tmp-{}", std::process::id()))
}

/// Preset names under `dir`: the stem of every non-hidden `*.json` file (a leftover atomic-write
/// temp file starts with `.` and is skipped), sorted. Empty — not an error — when `dir` doesn't
/// exist yet (no preset has been saved there).
pub fn list_json_names(dir: &Path) -> Result<Vec<String>, PresetError> {
    let mut names = Vec::new();
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(names),
        Err(e) => return Err(e.into()),
    };
    for entry in entries {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }
        let file_name = entry.file_name();
        let Some(file_name) = file_name.to_str() else {
            continue;
        };
        if file_name.starts_with('.') {
            continue;
        }
        if let Some(name) = file_name.strip_suffix(".json") {
            names.push(name.to_owned());
        }
    }
    names.sort();
    Ok(names)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_dir(tag: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "vox-presets-test-{tag}-{}-{nanos}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn write_atomic_round_trips_and_leaves_no_temp_file() {
        let dir = tmp_dir("atomic");
        let path = dir.join("Podcast voice.json");
        write_atomic(&path, br#"{"hello":"world"}"#).unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            r#"{"hello":"world"}"#
        );
        let leftovers: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(leftovers, vec![path.file_name().unwrap().to_owned()]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_atomic_overwrites_an_existing_file_without_a_partial_write_being_visible() {
        let dir = tmp_dir("atomic-overwrite");
        let path = dir.join("preset.json");
        write_atomic(&path, b"{\"v\":1}").unwrap();
        write_atomic(&path, b"{\"v\":2}").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{\"v\":2}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A crash between the temp-file write and the rename leaves a hidden temp file next to a
    /// (possibly absent) real preset file; a listing must ignore it rather than surface a
    /// half-written or empty "preset".
    #[test]
    fn a_simulated_crash_leaves_a_temp_file_that_listing_ignores() {
        let dir = tmp_dir("crash");
        // A real, fully-written preset alongside the crash leftover.
        write_atomic(&dir.join("Real preset.json"), b"{}").unwrap();
        // Simulate the crash: create the hidden temp file `write_atomic` would have written,
        // then never rename it (as if the process died first).
        std::fs::write(
            dir.join(format!(".Crashed preset.json.tmp-{}", std::process::id())),
            b"{ incomplete",
        )
        .unwrap();

        let names = list_json_names(&dir).unwrap();
        assert_eq!(names, vec!["Real preset".to_owned()]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn listing_a_directory_that_does_not_exist_yet_is_empty_not_an_error() {
        let dir = tmp_dir("missing").join("never-created");
        assert_eq!(list_json_names(&dir).unwrap(), Vec::<String>::new());
    }
}

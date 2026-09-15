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

/// True if `file_name` is a leftover [`write_atomic`] temp file — i.e. it matches exactly the
/// pattern [`temp_path`] produces (`.<original file name>.tmp-<pid>`): starts with `.` and ends
/// with `.tmp-` followed by one or more ASCII digits. This is deliberately narrower than "starts
/// with `.`": a sanitized preset name never starts with `.` (`name::sanitize_preset_name`
/// replaces leading dots), so in practice this only ever matches real crash leftovers, not a
/// preset whose sanitized name happens to start with a dot.
fn is_atomic_temp_leftover(file_name: &str) -> bool {
    if !file_name.starts_with('.') {
        return false;
    }
    match file_name.rsplit_once(".tmp-") {
        Some((_, pid)) => !pid.is_empty() && pid.bytes().all(|b| b.is_ascii_digit()),
        None => false,
    }
}

/// Preset names under `dir`: the stem of every `*.json` file that isn't a leftover atomic-write
/// temp file (see [`is_atomic_temp_leftover`]), sorted. Empty — not an error — when `dir` doesn't
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
        if is_atomic_temp_leftover(file_name) {
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

    /// H-33: temp-file detection used to be "any leading `.`", which also hid a legitimately
    /// dot-prefixed `.json` file (e.g. one written by a future/other tool, or before
    /// `name::sanitize_preset_name` started stripping leading dots). It must match only the exact
    /// shape `temp_path` produces: `.<name>.tmp-<all-digits pid>`.
    #[test]
    fn only_the_exact_atomic_temp_pattern_is_hidden_not_every_dot_prefixed_file() {
        assert!(is_atomic_temp_leftover(&format!(
            ".Real.json.tmp-{}",
            std::process::id()
        )));
        // Not the pattern write_atomic produces: no ".tmp-" marker, or a non-numeric/empty
        // suffix after it, or no leading dot at all.
        for not_a_leftover in [
            ".hidden.json",
            "..evil.json",
            ".tmp-not-a-pid.json",
            ".Foo.json.tmp-",
            ".Foo.json.tmp-12a",
            "Real.json.tmp-123",
        ] {
            assert!(
                !is_atomic_temp_leftover(not_a_leftover),
                "{not_a_leftover:?} should not be treated as a temp leftover"
            );
        }
    }

    #[test]
    fn a_dot_prefixed_json_file_that_is_not_a_temp_leftover_is_listed() {
        let dir = tmp_dir("dot-prefixed-real-file");
        // Not something sanitize_preset_name would produce any more (H-33), but list_json_names
        // must not conflate "starts with a dot" with "is a crash leftover" — only the exact
        // write_atomic temp pattern is special.
        std::fs::write(dir.join("..evil.json"), b"{}").unwrap();
        let names = list_json_names(&dir).unwrap();
        assert_eq!(names, vec!["..evil".to_owned()]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

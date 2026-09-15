//! Persisted "flagged" crash counters for out-of-process plugins (T-804, ADR-008 §5): a runtime
//! crash (as opposed to a scan-time crash, [`crate::blocklist`]) never blocklists a plugin by
//! itself — it just increments a counter the plugin manager shows as a warning badge, and the
//! user decides whether to block it. Keyed by module id (`"clap:<id>"`), not by file, so it
//! survives the same plugin being found at a different path later. Persisted next to the scan
//! cache (`plugin-health.json`), write-then-rename.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Mutex, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

const HEALTH_VERSION: u32 = 1;

/// One module's crash flag.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CrashFlag {
    /// Times a sandboxed instance of this module has faulted at runtime (crashed, hung, exited
    /// unexpectedly or reported a fatal error — every [`crate::SandboxFault`] counts once each).
    pub crash_count: u32,
    /// The most recent one (Unix ms).
    pub last_unix_ms: u64,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct HealthFile {
    version: u32,
    flags: BTreeMap<String, CrashFlag>,
}

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or_default()
}

fn read_file(path: &std::path::Path) -> BTreeMap<String, CrashFlag> {
    std::fs::read(path)
        .ok()
        .and_then(|b| serde_json::from_slice::<HealthFile>(&b).ok())
        .filter(|f| f.version == HEALTH_VERSION)
        .map(|f| f.flags)
        .unwrap_or_default()
}

fn write_file(path: &std::path::Path, flags: &BTreeMap<String, CrashFlag>) {
    let file = HealthFile {
        version: HEALTH_VERSION,
        flags: flags.clone(),
    };
    let Ok(json) = serde_json::to_vec_pretty(&file) else {
        return;
    };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let tmp = path.with_extension("tmp");
    if std::fs::write(&tmp, json).is_ok() {
        let _ = std::fs::rename(&tmp, path);
    }
}

/// The crash-flag store, shared (through an `Arc`, [`crate::SandboxOptions::health`]) between
/// every sandbox a process spawns.
#[derive(Debug, Default)]
pub struct HealthStore {
    path: Option<PathBuf>,
    flags: Mutex<BTreeMap<String, CrashFlag>>,
}

impl HealthStore {
    /// Loads the store from `path` (`None`: in-memory only — tests, CLI, offline render).
    pub fn load(path: Option<PathBuf>) -> Self {
        let flags = path.as_deref().map(read_file).unwrap_or_default();
        Self {
            path,
            flags: Mutex::new(flags),
        }
    }

    /// Increments `module_id`'s crash count and persists it. Returns the new flag.
    pub fn record_crash(&self, module_id: &str) -> CrashFlag {
        let mut g = self.flags.lock().unwrap_or_else(PoisonError::into_inner);
        let flag = g.entry(module_id.to_owned()).or_default();
        flag.crash_count += 1;
        flag.last_unix_ms = now_unix_ms();
        let flag = *flag;
        if let Some(path) = &self.path {
            write_file(path, &g);
        }
        flag
    }

    /// `module_id`'s current flag (zeroed if it has never faulted).
    pub fn get(&self, module_id: &str) -> CrashFlag {
        self.flags
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(module_id)
            .copied()
            .unwrap_or_default()
    }

    /// Clears `module_id`'s flag (e.g. after the user dismisses/blocks it).
    pub fn clear(&self, module_id: &str) {
        let mut g = self.flags.lock().unwrap_or_else(PoisonError::into_inner);
        if g.remove(module_id).is_some()
            && let Some(path) = &self.path
        {
            write_file(path, &g);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(label: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir =
            std::env::temp_dir().join(format!("pv-health-{label}-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn crashes_accumulate_and_persist() {
        let dir = temp_dir("basic");
        let path = dir.join("health.json");
        let store = HealthStore::load(Some(path.clone()));
        assert_eq!(store.get("clap:x").crash_count, 0);
        store.record_crash("clap:x");
        let flag = store.record_crash("clap:x");
        assert_eq!(flag.crash_count, 2);

        let reloaded = HealthStore::load(Some(path));
        assert_eq!(reloaded.get("clap:x").crash_count, 2);
        assert_eq!(reloaded.get("clap:other").crash_count, 0);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn clear_resets_the_flag() {
        let dir = temp_dir("clear");
        let path = dir.join("health.json");
        let store = HealthStore::load(Some(path.clone()));
        store.record_crash("clap:x");
        store.clear("clap:x");
        assert_eq!(store.get("clap:x").crash_count, 0);
        assert_eq!(HealthStore::load(Some(path)).get("clap:x").crash_count, 0);

        std::fs::remove_dir_all(&dir).ok();
    }
}

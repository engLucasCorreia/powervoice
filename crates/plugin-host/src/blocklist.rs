//! The plugin blocklist (T-804, ADR-008 §5 + Amendment 4): a file that crashes or times out
//! while being scanned is auto-blocklisted, keyed by path + size + mtime + a content hash, so it
//! clears itself the moment the file changes; the user can also block/unblock a plugin manually
//! (the same key, same auto-clear). Blocklisted files are skipped by later scans (never spawn a
//! sandbox for them) and never reach the registry. Persisted next to the scan cache
//! (`plugin-blocklist.json`), write-then-rename (same convention as [`crate::scan::write_cache`]).

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::scan::{Stamp, stamp};

/// Blocklist file format version.
const BLOCKLIST_VERSION: u32 = 1;

/// Why a file is blocklisted.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BlockReason {
    /// The sandbox crashed while scanning it (ADR-008 §5).
    Crashed,
    /// The scan timed out (ADR-008 §4/§5).
    TimedOut,
    /// The user blocked it manually (`plugins_block`, T-804 item 6).
    Manual,
}

impl BlockReason {
    /// "‹file› ‹reason›" (the plugin manager's blocklist reason text).
    pub fn message(self) -> &'static str {
        match self {
            Self::Crashed => "crashed while being scanned",
            Self::TimedOut => "timed out while being scanned",
            Self::Manual => "blocked by the user",
        }
    }
}

/// One blocklisted file, as read back ([`Blocklist::check`]).
#[derive(Clone, Debug, PartialEq)]
pub struct BlockInfo {
    /// Why it was blocked.
    pub reason: BlockReason,
    /// When ([`std::time::SystemTime`]-derived Unix milliseconds).
    pub blocked_at_unix_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct Entry {
    path: String,
    #[serde(flatten)]
    stamp: Stamp,
    /// CRC32 over the file's bytes at block time (`None`: unreadable at the time). A `None`
    /// entry still clears on a stamp mismatch; it just can't also catch a same-stamp content
    /// swap (astronomically unlikely for a real plugin file, and never true of a test fixture
    /// unless it deliberately keeps size+mtime identical).
    hash: Option<u32>,
    reason: BlockReason,
    blocked_at_unix_ms: u64,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct BlocklistFile {
    version: u32,
    entries: Vec<Entry>,
}

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or_default()
}

/// CRC32 over `path`'s bytes, or `None` if it can't be read.
fn content_hash(path: &Path) -> Option<u32> {
    std::fs::read(path)
        .ok()
        .map(|bytes| crc32fast::hash(&bytes))
}

fn read_file(path: &Path) -> Vec<Entry> {
    std::fs::read(path)
        .ok()
        .and_then(|b| serde_json::from_slice::<BlocklistFile>(&b).ok())
        .filter(|f| f.version == BLOCKLIST_VERSION)
        .map(|f| f.entries)
        .unwrap_or_default()
}

fn write_file(path: &Path, entries: &[Entry]) {
    let file = BlocklistFile {
        version: BLOCKLIST_VERSION,
        entries: entries.to_vec(),
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

/// The plugin blocklist (in-memory, mirrored to disk on every change).
pub struct Blocklist {
    path: Option<PathBuf>,
    entries: Vec<Entry>,
}

impl Blocklist {
    /// Loads the blocklist from `path` (`None`: in-memory only, e.g. offline render/CLI).
    pub fn load(path: Option<PathBuf>) -> Self {
        let entries = path.as_deref().map(read_file).unwrap_or_default();
        Self { path, entries }
    }

    fn save(&self) {
        if let Some(path) = &self.path {
            write_file(path, &self.entries);
        }
    }

    fn key(path: &Path) -> String {
        path.to_string_lossy().into_owned()
    }

    /// Whether `path` is currently blocked. A stale entry (the file's size, mtime or content
    /// changed since it was blocked) is dropped as a side effect — "cleared when the file
    /// changes" (ADR-008 §5) — and this returns `None` for it, same as if it had never been
    /// blocked.
    pub fn check(&mut self, path: &Path) -> Option<BlockInfo> {
        let key = Self::key(path);
        let idx = self.entries.iter().position(|e| e.path == key)?;
        let still_matches = stamp(path).is_some_and(|st| {
            let e = &self.entries[idx];
            e.stamp == st && e.hash.is_none_or(|h| content_hash(path) == Some(h))
        });
        if still_matches {
            let e = &self.entries[idx];
            return Some(BlockInfo {
                reason: e.reason,
                blocked_at_unix_ms: e.blocked_at_unix_ms,
            });
        }
        self.entries.remove(idx);
        self.save();
        None
    }

    /// Blocks `path` for `reason`, replacing any existing entry (stats it now: a manual block of
    /// a file that no longer exists is a `None` stamp, so it can never "match" again — it simply
    /// never blocks, same as not blocking at all; callers should check the file exists first).
    pub fn block(&mut self, path: &Path, reason: BlockReason) {
        let Some(st) = stamp(path) else { return };
        let key = Self::key(path);
        self.entries.retain(|e| e.path != key);
        self.entries.push(Entry {
            path: key,
            stamp: st,
            hash: content_hash(path),
            reason,
            blocked_at_unix_ms: now_unix_ms(),
        });
        self.save();
    }

    /// Clears `path`'s entry, if any. `true` if one was removed.
    pub fn unblock(&mut self, path: &Path) -> bool {
        let key = Self::key(path);
        let before = self.entries.len();
        self.entries.retain(|e| e.path != key);
        let removed = self.entries.len() != before;
        if removed {
            self.save();
        }
        removed
    }

    /// Every blocked path and why, sorted by path.
    pub fn list(&self) -> Vec<(PathBuf, BlockInfo)> {
        let mut out: Vec<(PathBuf, BlockInfo)> = self
            .entries
            .iter()
            .map(|e| {
                (
                    PathBuf::from(&e.path),
                    BlockInfo {
                        reason: e.reason,
                        blocked_at_unix_ms: e.blocked_at_unix_ms,
                    },
                )
            })
            .collect();
        out.sort_by(|a, b| a.0.cmp(&b.0));
        out
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
        let dir = std::env::temp_dir().join(format!(
            "pv-blocklist-{label}-{}-{nanos}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_blocked_file_is_reported_until_it_changes() {
        let dir = temp_dir("basic");
        let file = dir.join("bad.clap");
        std::fs::write(&file, b"v1").unwrap();
        let store_path = dir.join("blocklist.json");

        let mut b = Blocklist::load(Some(store_path.clone()));
        assert!(b.check(&file).is_none());
        b.block(&file, BlockReason::Crashed);
        let info = b.check(&file).unwrap();
        assert_eq!(info.reason, BlockReason::Crashed);

        // Reloaded from disk: still blocked (persisted).
        let mut reloaded = Blocklist::load(Some(store_path.clone()));
        assert!(reloaded.check(&file).is_some());

        // Touching the content (mtime *and* bytes change) clears it.
        std::fs::write(&file, b"v2, longer content").unwrap();
        assert!(reloaded.check(&file).is_none());
        // The clearing was persisted too.
        let mut again = Blocklist::load(Some(store_path));
        assert!(again.check(&file).is_none());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn mtime_only_change_also_clears_the_entry() {
        let dir = temp_dir("mtime-only");
        let file = dir.join("bad.clap");
        std::fs::write(&file, b"same bytes").unwrap();
        let mut b = Blocklist::load(Some(dir.join("blocklist.json")));
        b.block(&file, BlockReason::TimedOut);
        assert!(b.check(&file).is_some());

        let later = SystemTime::now() + std::time::Duration::from_secs(5);
        std::fs::File::options()
            .write(true)
            .open(&file)
            .unwrap()
            .set_modified(later)
            .unwrap();
        assert!(b.check(&file).is_none(), "mtime bump alone must clear it");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn unblock_clears_a_manual_block() {
        let dir = temp_dir("unblock");
        let file = dir.join("bad.clap");
        std::fs::write(&file, b"x").unwrap();
        let mut b = Blocklist::load(Some(dir.join("blocklist.json")));
        b.block(&file, BlockReason::Manual);
        assert!(b.check(&file).is_some());
        assert!(b.unblock(&file));
        assert!(b.check(&file).is_none());
        assert!(!b.unblock(&file), "already gone");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn list_is_sorted_by_path() {
        let dir = temp_dir("list");
        let a = dir.join("a.clap");
        let z = dir.join("z.clap");
        std::fs::write(&a, b"a").unwrap();
        std::fs::write(&z, b"z").unwrap();
        let mut b = Blocklist::load(Some(dir.join("blocklist.json")));
        b.block(&z, BlockReason::Crashed);
        b.block(&a, BlockReason::Manual);
        let list = b.list();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].0, a);
        assert_eq!(list[1].0, z);

        std::fs::remove_dir_all(&dir).ok();
    }
}

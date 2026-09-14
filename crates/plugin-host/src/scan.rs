//! CLAP enumeration (T-803, ADR-008 §6 + Amendment 3): find the `.clap` files in the standard
//! paths, scan each **in its own `powervoice-sandbox --scan` process** (a file that crashes or
//! hangs while being scanned is reported, never fatal), and cache the results by path + size +
//! mtime so later starts don't load unchanged files again. The full scanner (background scans,
//! blocklist, UI) is T-804.
//!
//! Standard paths (the CLAP spec's `entry.h`):
//! - Linux/BSD: `~/.clap`, `/usr/lib/clap`;
//! - macOS: `~/Library/Audio/Plug-Ins/CLAP`, `/Library/Audio/Plug-Ins/CLAP`;
//! - Windows: `%LOCALAPPDATA%\Programs\Common\CLAP`, `%COMMONPROGRAMFILES%\CLAP`;
//!
//! plus every directory of `$CLAP_PATH` (OS path-list syntax), searched first.

use std::collections::HashSet;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError, mpsc};
use std::time::{Duration, Instant, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use vox_module_api::{ModuleFactory, features};
use vox_sandbox_ipc::protocol::ScanReply;
pub use vox_sandbox_ipc::protocol::ScannedPlugin;

use crate::factory::{SandboxFactory, SandboxOptions, SandboxSpec, clap_spec};

/// ADR-008 §4: a scan may take 30 s.
pub const SCAN_TIMEOUT: Duration = Duration::from_secs(30);
/// Directory depth searched below each CLAP path.
const MAX_DEPTH: usize = 8;
/// Cache file format version.
const CACHE_VERSION: u32 = 1;

/// How to scan.
#[derive(Clone, Debug)]
pub struct ScanOptions {
    /// The `powervoice-sandbox` executable.
    pub binary: PathBuf,
    /// The cache file (`None`: no cache).
    pub cache: Option<PathBuf>,
    /// Per-file timeout.
    pub timeout: Duration,
    /// Parallel scans.
    pub jobs: usize,
}

impl ScanOptions {
    /// Defaults: no cache, [`SCAN_TIMEOUT`], half the cores in parallel (ADR-008 §6).
    pub fn new(binary: impl Into<PathBuf>) -> Self {
        let cores = std::thread::available_parallelism().map_or(2, |n| n.get());
        Self {
            binary: binary.into(),
            cache: None,
            timeout: SCAN_TIMEOUT,
            jobs: (cores / 2).max(1),
        }
    }
}

/// A plugin found in a file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FoundPlugin {
    /// The `.clap` file.
    pub path: PathBuf,
    /// Its descriptor.
    pub plugin: ScannedPlugin,
}

/// A file that couldn't be scanned.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScanFailure {
    /// The `.clap` file.
    pub path: PathBuf,
    /// Why ("crashed while being scanned (signal 6)", "is not a CLAP plugin", …).
    pub message: String,
}

/// The result of a scan.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ScanOutcome {
    /// Every plugin of every scanned file, in file order.
    pub plugins: Vec<FoundPlugin>,
    /// Files that failed.
    pub failures: Vec<ScanFailure>,
    /// Files scanned in a sandbox this time.
    pub scanned: usize,
    /// Files answered from the cache.
    pub cached: usize,
}

/// The CLAP search paths for this OS, `$CLAP_PATH` first, without duplicates.
pub fn clap_search_paths() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::env::var_os("CLAP_PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    let env_dir = |var: &str, rest: &[&str]| {
        std::env::var_os(var).map(|base| rest.iter().fold(PathBuf::from(base), |p, r| p.join(r)))
    };
    #[cfg(target_os = "macos")]
    {
        dirs.extend(env_dir("HOME", &["Library", "Audio", "Plug-Ins", "CLAP"]));
        dirs.push(PathBuf::from("/Library/Audio/Plug-Ins/CLAP"));
    }
    #[cfg(windows)]
    {
        dirs.extend(env_dir("LOCALAPPDATA", &["Programs", "Common", "CLAP"]));
        dirs.extend(env_dir("COMMONPROGRAMFILES", &["CLAP"]));
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        dirs.extend(env_dir("HOME", &[".clap"]));
        dirs.push(PathBuf::from("/usr/lib/clap"));
    }
    let mut seen = HashSet::new();
    dirs.retain(|d| !d.as_os_str().is_empty() && seen.insert(d.clone()));
    dirs
}

fn is_clap(path: &Path) -> bool {
    path.extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("clap"))
}

fn walk(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        // `metadata` follows symlinks (plugins are often linked into ~/.clap).
        let Ok(meta) = std::fs::metadata(&path) else {
            continue;
        };
        if is_clap(&path) {
            // A macOS bundle is a directory; elsewhere the `.clap` is the library file.
            if meta.is_file() || cfg!(target_os = "macos") {
                out.push(path);
            }
        } else if meta.is_dir() && depth < MAX_DEPTH {
            walk(&path, depth + 1, out);
        }
    }
}

/// Every `.clap` file below `dirs` (recursively), sorted, without duplicates.
pub fn find_clap_files(dirs: &[PathBuf]) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for d in dirs {
        walk(d, 0, &mut out);
    }
    let mut seen = HashSet::new();
    out.retain(|p| seen.insert(std::fs::canonicalize(p).unwrap_or_else(|_| p.clone())));
    out.sort();
    out
}

/// A file's plugins, or why it couldn't be scanned.
type FileResult = Result<Vec<ScannedPlugin>, String>;

/// Size and modification time: the cache key next to the path.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Stamp {
    size: u64,
    mtime_s: u64,
    mtime_ns: u32,
}

fn stamp(path: &Path) -> Option<Stamp> {
    let meta = std::fs::metadata(path).ok()?;
    let t = meta.modified().ok()?.duration_since(UNIX_EPOCH).ok()?;
    Some(Stamp {
        size: meta.len(),
        mtime_s: t.as_secs(),
        mtime_ns: t.subsec_nanos(),
    })
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct CacheEntry {
    path: String,
    #[serde(flatten)]
    stamp: Stamp,
    plugins: Vec<ScannedPlugin>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct CacheFile {
    version: u32,
    entries: Vec<CacheEntry>,
}

fn read_cache(path: &Path) -> Vec<CacheEntry> {
    std::fs::read(path)
        .ok()
        .and_then(|b| serde_json::from_slice::<CacheFile>(&b).ok())
        .filter(|c| c.version == CACHE_VERSION)
        .map(|c| c.entries)
        .unwrap_or_default()
}

fn write_cache(path: &Path, entries: Vec<CacheEntry>) {
    let file = CacheFile {
        version: CACHE_VERSION,
        entries,
    };
    let Ok(json) = serde_json::to_vec_pretty(&file) else {
        return;
    };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    // Write-then-rename: a crash mid-write never leaves a torn cache.
    let tmp = path.with_extension("tmp");
    if std::fs::write(&tmp, json).is_ok() {
        let _ = std::fs::rename(&tmp, path);
    }
}

/// What an exit status means for a scan that printed nothing usable.
fn describe_exit(status: std::process::ExitStatus) -> String {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(sig) = status.signal() {
            return format!("crashed while being scanned (signal {sig})");
        }
    }
    match status.code() {
        Some(code) => format!("the scan failed (exit code {code})"),
        None => "crashed while being scanned".to_owned(),
    }
}

/// Scans one file in a `powervoice-sandbox --scan` process (killed after `timeout`).
pub fn scan_file(
    binary: &Path,
    path: &Path,
    timeout: Duration,
) -> Result<Vec<ScannedPlugin>, String> {
    let mut child = Command::new(binary)
        .arg("--scan")
        .arg(path)
        .arg("--format")
        .arg("clap")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|e| format!("couldn't start the plugin sandbox: {e}"))?;
    let Some(mut stdout) = child.stdout.take() else {
        let _ = child.kill();
        let _ = child.wait();
        return Err("couldn't read the scan result".into());
    };
    let (tx, rx) = mpsc::channel();
    std::thread::Builder::new()
        .name("clap-scan-rx".into())
        .spawn(move || {
            let mut buf = Vec::new();
            let _ = stdout.read_to_end(&mut buf);
            let _ = tx.send(buf);
        })
        .map_err(|e| format!("couldn't read the scan result: {e}"))?;
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!(
                    "didn't finish scanning within {} s and was stopped",
                    timeout.as_secs()
                ));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(5)),
            Err(e) => return Err(format!("lost the scan process: {e}")),
        }
    };
    let out = rx.recv_timeout(Duration::from_secs(1)).unwrap_or_default();
    match serde_json::from_slice::<ScanReply>(&out) {
        Ok(ScanReply::Ok(report)) if status.success() => Ok(report.plugins),
        Ok(ScanReply::Error(message)) => Err(message),
        _ => Err(describe_exit(status)),
    }
}

/// Scans `files` (cache first, then sandboxed scans in parallel) and refreshes the cache.
pub fn scan_clap_files(files: &[PathBuf], options: &ScanOptions) -> ScanOutcome {
    let cache = options.cache.as_deref().map(read_cache).unwrap_or_default();
    let mut outcome = ScanOutcome::default();
    // Per file: its stamp and, once known, its plugins (or its failure).
    let mut slots: Vec<(Option<Stamp>, Option<FileResult>)> = files
        .iter()
        .map(|f| {
            let st = stamp(f);
            let key = f.to_string_lossy();
            let hit = st.and_then(|st| {
                cache
                    .iter()
                    .find(|e| e.path == key && e.stamp == st)
                    .map(|e| Ok(e.plugins.clone()))
            });
            (st, hit)
        })
        .collect();
    let todo: Vec<usize> = (0..files.len()).filter(|&i| slots[i].1.is_none()).collect();
    outcome.cached = files.len() - todo.len();
    outcome.scanned = todo.len();
    let results: Arc<Mutex<Vec<(usize, FileResult)>>> = Arc::new(Mutex::new(Vec::new()));
    let next = AtomicUsize::new(0);
    std::thread::scope(|s| {
        for _ in 0..options.jobs.clamp(1, todo.len().max(1)) {
            s.spawn(|| {
                loop {
                    let k = next.fetch_add(1, Ordering::Relaxed);
                    let Some(&i) = todo.get(k) else {
                        return;
                    };
                    let r = scan_file(&options.binary, &files[i], options.timeout);
                    results
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .push((i, r));
                }
            });
        }
    });
    let results = std::mem::take(&mut *results.lock().unwrap_or_else(PoisonError::into_inner));
    for (i, r) in results {
        slots[i].1 = Some(r);
    }
    let mut entries = Vec::new();
    for (file, (st, result)) in files.iter().zip(slots) {
        match result {
            Some(Ok(plugins)) => {
                if let Some(stamp) = st {
                    entries.push(CacheEntry {
                        path: file.to_string_lossy().into_owned(),
                        stamp,
                        plugins: plugins.clone(),
                    });
                }
                outcome
                    .plugins
                    .extend(plugins.into_iter().map(|plugin| FoundPlugin {
                        path: file.clone(),
                        plugin,
                    }));
            }
            Some(Err(message)) => outcome.failures.push(ScanFailure {
                path: file.clone(),
                message,
            }),
            None => {}
        }
    }
    // Failures aren't cached: a transient failure retries at the next start (T-804 adds the
    // blocklist for files that keep crashing).
    if let Some(cache) = &options.cache {
        write_cache(cache, entries);
    }
    outcome
}

/// Whether a scanned plugin is an audio effect the rack can host (no instruments or note
/// effects).
pub fn is_effect(p: &ScannedPlugin) -> bool {
    let has = |f: &str| p.features.iter().any(|x| x == f);
    has(features::AUDIO_EFFECT) && !has("instrument") && !has("note-effect")
}

/// The specs of every audio effect in `outcome`; the first file wins for a duplicate id.
pub fn effect_specs(outcome: &ScanOutcome) -> Vec<SandboxSpec> {
    let mut seen = HashSet::new();
    outcome
        .plugins
        .iter()
        .filter(|f| is_effect(&f.plugin) && seen.insert(f.plugin.id.clone()))
        .map(|f| clap_spec(&f.path, &f.plugin))
        .collect()
}

/// A `SandboxFactory` per spec.
pub fn factories(specs: &[SandboxSpec], options: &SandboxOptions) -> Vec<Arc<dyn ModuleFactory>> {
    specs
        .iter()
        .map(|s| {
            Arc::new(SandboxFactory::new(s.clone(), options.clone())) as Arc<dyn ModuleFactory>
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plugin(id: &str, features: &[&str]) -> ScannedPlugin {
        ScannedPlugin {
            id: id.into(),
            name: id.into(),
            vendor: "V".into(),
            version: "1.2".into(),
            description: String::new(),
            url: None,
            features: features.iter().map(|s| (*s).to_owned()).collect(),
        }
    }

    #[test]
    fn effects_are_filtered_and_deduplicated() {
        let found = |path: &str, p| FoundPlugin {
            path: PathBuf::from(path),
            plugin: p,
        };
        let outcome = ScanOutcome {
            plugins: vec![
                found(
                    "/a.clap",
                    plugin("com.x.eq", &["audio-effect", "equalizer"]),
                ),
                found("/a.clap", plugin("com.x.synth", &["instrument"])),
                found("/b.clap", plugin("com.x.eq", &["audio-effect"])),
                found(
                    "/b.clap",
                    plugin("com.x.arp", &["audio-effect", "note-effect"]),
                ),
            ],
            ..ScanOutcome::default()
        };
        let specs = effect_specs(&outcome);
        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0].descriptor.id, "clap:com.x.eq");
        assert_eq!(specs[0].descriptor.version.to_string(), "1.2.0");
        assert_eq!(specs[0].format, "clap");
        assert!(specs[0].plugin.contains("/a.clap"));
    }

    #[test]
    fn search_paths_include_clap_path_first() {
        // Only reads the environment; `CLAP_PATH` may or may not be set by the caller.
        let dirs = clap_search_paths();
        if let Some(p) = std::env::var_os("CLAP_PATH") {
            let first: Vec<PathBuf> = std::env::split_paths(&p).collect();
            assert_eq!(
                &dirs[..first.len().min(dirs.len())],
                &first[..first.len().min(dirs.len())]
            );
        }
        #[cfg(all(unix, not(target_os = "macos")))]
        assert!(dirs.contains(&PathBuf::from("/usr/lib/clap")));
    }
}

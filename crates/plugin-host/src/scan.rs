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

use crate::blocklist::{BlockReason, Blocklist};
use crate::factory::{SandboxFactory, SandboxOptions, SandboxSpec, clap_spec};

/// ADR-008 §4: a scan may take 30 s.
pub const SCAN_TIMEOUT: Duration = Duration::from_secs(30);
/// Directory depth searched below each CLAP path.
const MAX_DEPTH: usize = 8;
/// Cache file format version. T-804 (Amendment 4) bumped this from 1 to 2 when `ScannedPlugin`
/// grew richer fields (`param_count`, `main_input_channels`, `main_output_channels`): an old v1
/// cache has none of that data, so [`read_cache`] discards it wholesale rather than risk a
/// registry entry with zeroed ports that were never actually checked.
const CACHE_VERSION: u32 = 2;

/// How to scan.
#[derive(Clone, Debug)]
pub struct ScanOptions {
    /// The `powervoice-sandbox` executable.
    pub binary: PathBuf,
    /// The cache file (`None`: no cache).
    pub cache: Option<PathBuf>,
    /// The blocklist file (T-804, ADR-008 §5; `None`: no blocklist — every file is scanned, and
    /// a crash/timeout is never remembered).
    pub blocklist: Option<PathBuf>,
    /// Per-file timeout.
    pub timeout: Duration,
    /// Parallel scans.
    pub jobs: usize,
    /// Ignores the cache (`plugins_rescan`'s "full" flag, T-804 item 6): every file is scanned
    /// again in a sandbox, even an unchanged one. The cache is still refreshed afterwards.
    /// Blocklisted files are still skipped — a full rescan is not the same as unblocking.
    pub force: bool,
}

impl ScanOptions {
    /// Defaults: no cache, no blocklist, [`SCAN_TIMEOUT`], half the cores in parallel
    /// (ADR-008 §6).
    pub fn new(binary: impl Into<PathBuf>) -> Self {
        let cores = std::thread::available_parallelism().map_or(2, |n| n.get());
        Self {
            binary: binary.into(),
            cache: None,
            blocklist: None,
            timeout: SCAN_TIMEOUT,
            jobs: (cores / 2).max(1),
            force: false,
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

/// Whether a scan failure should auto-blocklist the file (ADR-008 §5: only a crash or a
/// timeout — never a merely-unreadable or not-a-plugin file, and never an environment problem
/// like a missing sandbox binary).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FailureKind {
    /// The sandbox process crashed (signal, or exited with garbage output).
    Crashed,
    /// The scan didn't finish within the timeout and was killed.
    TimedOut,
    /// Anything else (not a CLAP library, no factory, couldn't even start the sandbox, …).
    Other,
}

/// A file that couldn't be scanned.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScanFailure {
    /// The `.clap` file.
    pub path: PathBuf,
    /// Why ("crashed while being scanned (signal 6)", "is not a CLAP plugin", …).
    pub message: String,
    /// Whether this auto-blocklists the file.
    pub kind: FailureKind,
}

/// [`scan_file`]'s error.
#[derive(Clone, Debug, PartialEq, Eq)]
struct ScanFileError {
    message: String,
    kind: FailureKind,
}

impl ScanFileError {
    fn other(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            kind: FailureKind::Other,
        }
    }
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
    /// Files skipped because they're blocklisted (T-804, ADR-008 §5) — not scanned, not in
    /// `plugins`, not in `failures`.
    pub blocklisted: Vec<PathBuf>,
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
type FileResult = Result<Vec<ScannedPlugin>, ScanFileError>;

/// Size and modification time: the cache key next to the path (also half of the blocklist key,
/// [`crate::blocklist`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Stamp {
    pub(crate) size: u64,
    pub(crate) mtime_s: u64,
    pub(crate) mtime_ns: u32,
}

pub(crate) fn stamp(path: &Path) -> Option<Stamp> {
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
fn describe_exit(status: std::process::ExitStatus) -> ScanFileError {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(sig) = status.signal() {
            return ScanFileError {
                message: format!("crashed while being scanned (signal {sig})"),
                kind: FailureKind::Crashed,
            };
        }
    }
    match status.code() {
        Some(code) => ScanFileError::other(format!("the scan failed (exit code {code})")),
        None => ScanFileError {
            message: "crashed while being scanned".to_owned(),
            kind: FailureKind::Crashed,
        },
    }
}

/// Scans one file in a `powervoice-sandbox --scan` process (killed after `timeout`).
pub fn scan_file(
    binary: &Path,
    path: &Path,
    timeout: Duration,
) -> Result<Vec<ScannedPlugin>, String> {
    scan_file_inner(binary, path, timeout).map_err(|e| e.message)
}

fn scan_file_inner(
    binary: &Path,
    path: &Path,
    timeout: Duration,
) -> Result<Vec<ScannedPlugin>, ScanFileError> {
    let mut child = Command::new(binary)
        .arg("--scan")
        .arg(path)
        .arg("--format")
        .arg("clap")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|e| ScanFileError::other(format!("couldn't start the plugin sandbox: {e}")))?;
    let Some(mut stdout) = child.stdout.take() else {
        let _ = child.kill();
        let _ = child.wait();
        return Err(ScanFileError::other("couldn't read the scan result"));
    };
    let (tx, rx) = mpsc::channel();
    std::thread::Builder::new()
        .name("clap-scan-rx".into())
        .spawn(move || {
            let mut buf = Vec::new();
            let _ = stdout.read_to_end(&mut buf);
            let _ = tx.send(buf);
        })
        .map_err(|e| ScanFileError::other(format!("couldn't read the scan result: {e}")))?;
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(ScanFileError {
                    message: format!(
                        "didn't finish scanning within {} s and was stopped",
                        timeout.as_secs()
                    ),
                    kind: FailureKind::TimedOut,
                });
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(5)),
            Err(e) => return Err(ScanFileError::other(format!("lost the scan process: {e}"))),
        }
    };
    let out = rx.recv_timeout(Duration::from_secs(1)).unwrap_or_default();
    match serde_json::from_slice::<ScanReply>(&out) {
        Ok(ScanReply::Ok(report)) if status.success() => Ok(report.plugins),
        Ok(ScanReply::Error(message)) => Err(ScanFileError::other(message)),
        _ => Err(describe_exit(status)),
    }
}

/// Scans `files` (blocklist first, then cache, then sandboxed scans in parallel) and refreshes
/// the cache and blocklist.
pub fn scan_clap_files(files: &[PathBuf], options: &ScanOptions) -> ScanOutcome {
    scan_clap_files_with(files, options, |_, _, _| {})
}

/// [`scan_clap_files`], calling `on_progress(done, total, current_file)` once per **live**
/// (not-blocklisted) file as it's accounted for (T-804 item 1: `plugin_scan_progress`) — cache
/// hits are reported immediately (before any sandbox runs), then sandboxed scans as they finish.
/// `done` is a plain 1, 2, 3, … counter (a `fetch_add` result), so progress is always reported in
/// increasing order even though the underlying scans run in parallel and can finish in any file
/// order.
pub fn scan_clap_files_with(
    files: &[PathBuf],
    options: &ScanOptions,
    on_progress: impl Fn(usize, usize, &Path) + Sync,
) -> ScanOutcome {
    let mut blocklist = Blocklist::load(options.blocklist.clone());
    let mut outcome = ScanOutcome::default();
    // Blocklisted files never reach the cache lookup or a sandbox: ADR-008 §5 "skipped by later
    // scans", "not in the registry". `Blocklist::check` also clears a stale entry (the file
    // changed since it was blocked), so a changed file falls through to a normal scan below.
    let live: Vec<PathBuf> = files
        .iter()
        .filter(|f| match blocklist.check(f) {
            Some(_) => {
                outcome.blocklisted.push((*f).clone());
                false
            }
            None => true,
        })
        .cloned()
        .collect();
    let cache = options.cache.as_deref().map(read_cache).unwrap_or_default();
    // Per live file: its stamp and, once known, its plugins (or its failure).
    let mut slots: Vec<(Option<Stamp>, Option<FileResult>)> = live
        .iter()
        .map(|f| {
            let st = stamp(f);
            let key = f.to_string_lossy();
            let hit = if options.force {
                None
            } else {
                st.and_then(|st| {
                    cache
                        .iter()
                        .find(|e| e.path == key && e.stamp == st)
                        .map(|e| Ok(e.plugins.clone()))
                })
            };
            (st, hit)
        })
        .collect();
    let total = live.len();
    // A plain `Mutex<usize>` (not an `AtomicUsize`), so incrementing the counter and calling
    // `on_progress` happen as one step: two threads finishing "at the same time" still deliver
    // their calls one at a time, in counter order, never interleaved (T-804 item 1: "progress
    // events arrive in order" — an `AtomicUsize::fetch_add` alone only orders the *numbers*, not
    // the calls, since a thread can be preempted between incrementing and calling).
    let done = Mutex::new(0usize);
    let report = |path: &Path| {
        let mut g = done.lock().unwrap_or_else(PoisonError::into_inner);
        *g += 1;
        on_progress(*g, total, path);
    };
    for i in 0..live.len() {
        if slots[i].1.is_some() {
            report(&live[i]);
        }
    }
    let todo: Vec<usize> = (0..live.len()).filter(|&i| slots[i].1.is_none()).collect();
    outcome.cached = live.len() - todo.len();
    outcome.scanned = todo.len();
    let results: Arc<Mutex<Vec<(usize, FileResult)>>> = Arc::new(Mutex::new(Vec::new()));
    let next = AtomicUsize::new(0);
    let report = &report;
    std::thread::scope(|s| {
        for _ in 0..options.jobs.clamp(1, todo.len().max(1)) {
            s.spawn(|| {
                loop {
                    let k = next.fetch_add(1, Ordering::Relaxed);
                    let Some(&i) = todo.get(k) else {
                        return;
                    };
                    let r = scan_file_inner(&options.binary, &live[i], options.timeout);
                    report(&live[i]);
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
    for (file, (st, result)) in live.iter().zip(slots) {
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
            Some(Err(ScanFileError { message, kind })) => {
                // ADR-008 §5: only a crash or a timeout auto-blocklists — never a merely
                // unreadable file or an environment problem (missing sandbox binary, …).
                match kind {
                    FailureKind::Crashed => blocklist.block(file, BlockReason::Crashed),
                    FailureKind::TimedOut => blocklist.block(file, BlockReason::TimedOut),
                    FailureKind::Other => {}
                }
                outcome.failures.push(ScanFailure {
                    path: file.clone(),
                    message,
                    kind,
                });
            }
            None => {}
        }
    }
    // Failures aren't cached (retried at the next scan unless they got blocklisted above).
    if let Some(cache) = &options.cache {
        write_cache(cache, entries);
    }
    outcome
}

/// The effect specs of the plugins already known from the cache, without spawning a single
/// sandbox process (T-804 item 1: "start-up registers cached plugins immediately, then rescans
/// changed files in the background"). A file whose current size/mtime doesn't match its cache
/// entry (changed or new) is simply absent here — the background [`scan_clap_files`] picks it
/// up. Blocklisted files are excluded, same as a real scan.
pub fn cached_effect_specs(
    files: &[PathBuf],
    cache: Option<&Path>,
    blocklist: Option<&Path>,
) -> Vec<SandboxSpec> {
    let cache_entries = cache.map(read_cache).unwrap_or_default();
    let mut blocklist = Blocklist::load(blocklist.map(Path::to_path_buf));
    let mut outcome = ScanOutcome::default();
    for f in files {
        if blocklist.check(f).is_some() {
            continue;
        }
        let Some(st) = stamp(f) else { continue };
        let key = f.to_string_lossy();
        if let Some(e) = cache_entries
            .iter()
            .find(|e| e.path == key && e.stamp == st)
        {
            outcome
                .plugins
                .extend(e.plugins.iter().cloned().map(|plugin| FoundPlugin {
                    path: f.clone(),
                    plugin,
                }));
        }
    }
    effect_specs(&outcome)
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
            ..ScannedPlugin::default()
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

    fn temp_dir(label: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "pv-scan-test-{label}-{}-{nanos}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// No real sandbox is ever spawned for a blocklisted file (T-804, ADR-008 §5): it's excluded
    /// before the cache lookup and before any scan job runs, so a bogus `binary` path (which
    /// would error immediately if invoked) never gets the chance to.
    #[test]
    fn blocklisted_files_are_skipped_without_scanning() {
        let dir = temp_dir("blocklist-skip");
        let file = dir.join("bad.clap");
        std::fs::write(&file, b"x").unwrap();
        let mut blocklist = crate::blocklist::Blocklist::load(Some(dir.join("blocklist.json")));
        blocklist.block(&file, crate::blocklist::BlockReason::Crashed);
        drop(blocklist); // persisted; scan_clap_files loads its own copy.

        let mut options = ScanOptions::new(PathBuf::from("/nonexistent/powervoice-sandbox"));
        options.blocklist = Some(dir.join("blocklist.json"));
        let outcome = scan_clap_files(std::slice::from_ref(&file), &options);
        assert_eq!(outcome.blocklisted, vec![file]);
        assert_eq!((outcome.scanned, outcome.cached), (0, 0));
        assert!(outcome.plugins.is_empty());
        assert!(outcome.failures.is_empty());

        std::fs::remove_dir_all(&dir).ok();
    }

    /// A v1 cache (pre-T-804: no `param_count`/port fields) is discarded wholesale by the
    /// version bump to 2 — every file counts as a cache miss, not a stale hit with zeroed
    /// fields. (The rescan itself fails here — there's no real sandbox — but the cache-hit
    /// accounting is independent of that, and is what this test checks.)
    #[test]
    fn cache_schema_bump_invalidates_old_caches_cleanly() {
        let dir = temp_dir("cache-bump");
        let file = dir.join("plugin.clap");
        std::fs::write(&file, b"x").unwrap();
        let st = stamp(&file).unwrap();
        let cache_path = dir.join("cache.json");
        let v1_json = serde_json::json!({
            "version": 1,
            "entries": [{
                "path": file.to_string_lossy(),
                "size": st.size,
                "mtime_s": st.mtime_s,
                "mtime_ns": st.mtime_ns,
                "plugins": [{
                    "id": "com.x.old", "name": "Old", "vendor": "V", "version": "1",
                    "description": "", "url": null, "features": ["audio-effect"]
                }],
            }],
        });
        std::fs::write(&cache_path, serde_json::to_vec(&v1_json).unwrap()).unwrap();

        let mut options = ScanOptions::new(PathBuf::from("/nonexistent/powervoice-sandbox"));
        options.cache = Some(cache_path);
        let outcome = scan_clap_files(&[file], &options);
        assert_eq!(
            (outcome.cached, outcome.scanned),
            (0, 1),
            "the v1 cache must not produce a hit"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    /// `cached_effect_specs` (T-804 item 1's instant start-up path) never spawns a sandbox: a
    /// cached file registers immediately, an uncached one is silently absent, and a blocklisted
    /// one is excluded even though it's cached.
    #[test]
    fn cached_effect_specs_needs_no_sandbox() {
        let dir = temp_dir("cached-specs");
        let known = dir.join("known.clap");
        let unknown = dir.join("unknown.clap");
        let blocked = dir.join("blocked.clap");
        for f in [&known, &unknown, &blocked] {
            std::fs::write(f, b"x").unwrap();
        }
        let cache_path = dir.join("cache.json");
        let entry_for = |f: &Path, id: &str| {
            let st = stamp(f).unwrap();
            serde_json::json!({
                "path": f.to_string_lossy(), "size": st.size, "mtime_s": st.mtime_s,
                "mtime_ns": st.mtime_ns,
                "plugins": [{
                    "id": id, "name": id, "vendor": "V", "version": "1",
                    "description": "", "url": null, "features": ["audio-effect"]
                }],
            })
        };
        let cache = serde_json::json!({
            "version": CACHE_VERSION,
            "entries": [entry_for(&known, "com.x.known"), entry_for(&blocked, "com.x.blocked")],
        });
        std::fs::write(&cache_path, serde_json::to_vec(&cache).unwrap()).unwrap();
        let blocklist_path = dir.join("blocklist.json");
        let mut b = crate::blocklist::Blocklist::load(Some(blocklist_path.clone()));
        b.block(&blocked, crate::blocklist::BlockReason::Manual);

        let specs = cached_effect_specs(
            &[known, unknown, blocked],
            Some(&cache_path),
            Some(&blocklist_path),
        );
        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0].descriptor.id, "clap:com.x.known");

        std::fs::remove_dir_all(&dir).ok();
    }

    /// `ScanOptions::force` bypasses even a fresh (current-version) cache hit.
    #[test]
    fn force_ignores_a_fresh_cache_hit() {
        let dir = temp_dir("force");
        let file = dir.join("plugin.clap");
        std::fs::write(&file, b"x").unwrap();
        let cache_path = dir.join("cache.json");
        let mut options = ScanOptions::new(PathBuf::from("/nonexistent/powervoice-sandbox"));
        options.cache = Some(cache_path.clone());
        // First pass: fails to scan (no real sandbox), so nothing is cached.
        let _ = scan_clap_files(std::slice::from_ref(&file), &options);
        // Seed a hand-written cache entry so the second pass would normally hit it.
        let st = stamp(&file).unwrap();
        let seeded = serde_json::json!({
            "version": CACHE_VERSION,
            "entries": [{
                "path": file.to_string_lossy(),
                "size": st.size,
                "mtime_s": st.mtime_s,
                "mtime_ns": st.mtime_ns,
                "plugins": [],
            }],
        });
        std::fs::write(&cache_path, serde_json::to_vec(&seeded).unwrap()).unwrap();

        let normal = scan_clap_files(std::slice::from_ref(&file), &options);
        assert_eq!((normal.cached, normal.scanned), (1, 0));

        options.force = true;
        let forced = scan_clap_files(&[file], &options);
        assert_eq!((forced.cached, forced.scanned), (0, 1));

        std::fs::remove_dir_all(&dir).ok();
    }
}

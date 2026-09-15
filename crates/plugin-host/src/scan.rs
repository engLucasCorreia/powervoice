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
//!
//! **VST3** (T-806, ADR-008 Amendment 6): `.vst3` bundles (directories, never searched inside;
//! Windows' legacy single files too) in `$VST3_PATH` then the SDK's standard paths —
//! - Linux/BSD: `~/.vst3`, `/usr/lib/vst3`, `/usr/local/lib/vst3`;
//! - macOS: `~/Library/Audio/Plug-Ins/VST3`, `/Library/Audio/Plug-Ins/VST3`;
//! - Windows: `%LOCALAPPDATA%\Programs\Common\VST3`, `%COMMONPROGRAMFILES%\VST3`.
//!
//! A bundle with `Contents/Resources/moduleinfo.json` (and a binary for this platform) is
//! **indexed from that file without loading any code** (ADR-008 §6); any other bundle is scanned
//! in its own `powervoice-sandbox --scan <bundle> --format vst3`. Every scan function here takes
//! files of any format (the format follows the extension); cache entries and blocklist entries
//! carry it.
//!
//! **LV2** (T-807, ADR-008 Amendment 8): `.lv2` bundle directories (never searched inside) whose
//! `manifest.ttl` declares a plugin — LV2's specification bundles are skipped without a sandbox —
//! in `$LV2_PATH` then lilv's standard paths:
//! - Linux/BSD: `~/.lv2`, `/usr/local/lib/lv2`, `/usr/lib/lv2` (and the `lib64` variants);
//! - macOS: `~/Library/Audio/Plug-Ins/LV2`, `~/.lv2`, `/usr/local/lib/lv2`,
//!   `/Library/Audio/Plug-Ins/LV2`;
//! - Windows: none (the sandbox hosts LV2 on unix only).
//!
//! Each bundle is scanned in its own `powervoice-sandbox --scan <bundle> --format lv2`; a
//! bundle's stamp covers every file in it.
//!
//! **JSFX** (T-808, ADR-008 Amendment 11): scripts — `.jsfx` files, and extensionless files whose
//! header has a `desc:` line ([`vox_sandbox_ipc::jsfx::is_script`]; `.jsfx-inc` imports never) —
//! in PowerVoice's own JSFX folder (the install folder, [`crate::install::user_jsfx_dir`]), then
//! REAPER's effects folder if it exists ([`jsfx_search_paths`]), then the custom folders. Each
//! script is scanned in its own `powervoice-sandbox --scan <script> --format jsfx`. Unix only,
//! like the backend.

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

use vox_sandbox_ipc::vst3::{AUDIO_MODULE_CLASS, binary_path, moduleinfo_path};

use crate::blocklist::{BlockReason, Blocklist};
use crate::factory::{
    CLAP_FORMAT, JSFX_FORMAT, LV2_FORMAT, SandboxFactory, SandboxOptions, SandboxSpec, VST3_FORMAT,
    clap_spec, jsfx_spec, lv2_spec, vst3_spec,
};

/// ADR-008 §4: a scan may take 30 s.
pub const SCAN_TIMEOUT: Duration = Duration::from_secs(30);
/// Directory depth searched below each CLAP path.
const MAX_DEPTH: usize = 8;
/// Cache file format version. T-804 (Amendment 4) bumped this from 1 to 2 when `ScannedPlugin`
/// grew richer fields (`param_count`, `main_input_channels`, `main_output_channels`): an old v1
/// cache has none of that data, so [`read_cache`] discards it wholesale rather than risk a
/// registry entry with zeroed ports that were never actually checked.
const CACHE_VERSION: u32 = 2;

/// The cache file's name (H-34): renamed from [`LEGACY_CACHE_FILE_NAME`] once VST3 (T-806)
/// started sharing it — the old name only made sense back when it held CLAP results alone.
pub const CACHE_FILE_NAME: &str = "plugin-scan.json";

/// The cache file's name before H-34 (T-803 through T-806). A leftover file under this name is
/// migrated to [`CACHE_FILE_NAME`] once, by [`migrate_cache_file_name`].
pub const LEGACY_CACHE_FILE_NAME: &str = "clap-scan.json";

/// One-shot migration (H-34): if `new_path` (expected to end in [`CACHE_FILE_NAME`]) doesn't
/// exist yet, but a file named [`LEGACY_CACHE_FILE_NAME`] sits next to it, that file is renamed
/// into place — so a cache built before the rename survives it instead of forcing a full
/// rescan. A no-op when there's nothing to migrate (already migrated, first run, or no cache
/// directory yet); never overwrites an existing `new_path`.
pub fn migrate_cache_file_name(new_path: &Path) {
    if new_path.exists() {
        return;
    }
    let Some(dir) = new_path.parent() else {
        return;
    };
    let old_path = dir.join(LEGACY_CACHE_FILE_NAME);
    if old_path.is_file() {
        let _ = std::fs::rename(&old_path, new_path);
    }
}

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
    /// VST3 bundles indexed from their `moduleinfo.json` this time, without a sandbox (T-806;
    /// not counted in `scanned`).
    pub indexed: usize,
}

/// A plugin file format the scanner knows (T-806).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PluginFormat {
    /// `.clap` files (macOS: bundles).
    Clap,
    /// `.vst3` bundles (Windows: also legacy single files).
    Vst3,
    /// `.lv2` bundle directories (T-807).
    Lv2,
    /// JSFX scripts: `.jsfx` files, or extensionless files with a `desc:` line (T-808).
    Jsfx,
}

impl PluginFormat {
    /// The sandbox backend's name (`"clap"`, `"vst3"`, `"lv2"`, `"jsfx"`).
    pub fn name(self) -> &'static str {
        match self {
            Self::Clap => CLAP_FORMAT,
            Self::Vst3 => VST3_FORMAT,
            Self::Lv2 => LV2_FORMAT,
            Self::Jsfx => JSFX_FORMAT,
        }
    }

    /// The format of a plugin path: by its extension (any case), or — an extensionless file —
    /// JSFX when its header has a `desc:` line (T-808; reads the file's beginning).
    pub fn of_path(path: &Path) -> Option<Self> {
        match Self::of_extension(path) {
            Some(f) => Some(f),
            None if path.extension().is_none() && vox_sandbox_ipc::jsfx::is_script(path) => {
                Some(Self::Jsfx)
            }
            None => None,
        }
    }

    /// The format of a plugin path by its extension alone (any case; no file access).
    pub fn of_extension(path: &Path) -> Option<Self> {
        let ext = path.extension()?;
        if ext.eq_ignore_ascii_case("clap") {
            Some(Self::Clap)
        } else if ext.eq_ignore_ascii_case("vst3") {
            Some(Self::Vst3)
        } else if ext.eq_ignore_ascii_case("lv2") {
            Some(Self::Lv2)
        } else if ext.eq_ignore_ascii_case(vox_sandbox_ipc::jsfx::EXTENSION) {
            Some(Self::Jsfx)
        } else {
            None
        }
    }
}

/// [`PluginFormat::of_path`]'s backend name (`None`: not a plugin path).
pub fn format_of_path(path: &Path) -> Option<&'static str> {
    PluginFormat::of_path(path).map(PluginFormat::name)
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

/// The VST3 search paths for this OS (the SDK's standard locations), `$VST3_PATH` first, without
/// duplicates (T-806).
pub fn vst3_search_paths() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::env::var_os("VST3_PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    let env_dir = |var: &str, rest: &[&str]| {
        std::env::var_os(var).map(|base| rest.iter().fold(PathBuf::from(base), |p, r| p.join(r)))
    };
    #[cfg(target_os = "macos")]
    {
        dirs.extend(env_dir("HOME", &["Library", "Audio", "Plug-Ins", "VST3"]));
        dirs.push(PathBuf::from("/Library/Audio/Plug-Ins/VST3"));
    }
    #[cfg(windows)]
    {
        dirs.extend(env_dir("LOCALAPPDATA", &["Programs", "Common", "VST3"]));
        dirs.extend(env_dir("COMMONPROGRAMFILES", &["VST3"]));
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        dirs.extend(env_dir("HOME", &[".vst3"]));
        dirs.push(PathBuf::from("/usr/lib/vst3"));
        dirs.push(PathBuf::from("/usr/local/lib/vst3"));
    }
    let mut seen = HashSet::new();
    dirs.retain(|d| !d.as_os_str().is_empty() && seen.insert(d.clone()));
    dirs
}

/// The LV2 search paths for this OS (lilv's defaults), `$LV2_PATH` first, without duplicates
/// (T-807). Empty on Windows: the sandbox hosts LV2 on unix only.
pub fn lv2_search_paths() -> Vec<PathBuf> {
    if cfg!(windows) {
        return Vec::new();
    }
    let mut dirs: Vec<PathBuf> = std::env::var_os("LV2_PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    let home = std::env::var_os("HOME").map(PathBuf::from);
    #[cfg(target_os = "macos")]
    {
        dirs.extend(
            home.as_ref()
                .map(|h| h.join("Library").join("Audio").join("Plug-Ins").join("LV2")),
        );
        dirs.extend(home.as_ref().map(|h| h.join(".lv2")));
        dirs.push(PathBuf::from("/usr/local/lib/lv2"));
        dirs.push(PathBuf::from("/Library/Audio/Plug-Ins/LV2"));
    }
    #[cfg(not(target_os = "macos"))]
    {
        dirs.extend(home.as_ref().map(|h| h.join(".lv2")));
        for d in [
            "/usr/local/lib/lv2",
            "/usr/lib/lv2",
            "/usr/local/lib64/lv2",
            "/usr/lib64/lv2",
        ] {
            dirs.push(PathBuf::from(d));
        }
    }
    let mut seen = HashSet::new();
    dirs.retain(|d| !d.as_os_str().is_empty() && seen.insert(d.clone()));
    dirs
}

/// Whether this build hosts JSFX (the sandbox's backend is unix-only, T-808).
pub const JSFX_SUPPORTED: bool = cfg!(unix);

/// REAPER's effects folder for this OS, when it exists (T-808): JSFX scripts installed with
/// REAPER (or ReaPack) are offered too. Linux `$XDG_CONFIG_HOME/REAPER/Effects` (default
/// `~/.config/REAPER/Effects`), macOS `~/Library/Application Support/REAPER/Effects`. Empty on
/// Windows: the sandbox hosts JSFX on unix only.
pub fn jsfx_search_paths() -> Vec<PathBuf> {
    if !JSFX_SUPPORTED {
        return Vec::new();
    }
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|h| h.is_absolute());
    let mut dirs = Vec::new();
    #[cfg(target_os = "macos")]
    dirs.extend(home.map(|h| {
        h.join("Library")
            .join("Application Support")
            .join("REAPER")
            .join("Effects")
    }));
    #[cfg(not(target_os = "macos"))]
    {
        let config = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|c| c.is_absolute())
            .or_else(|| home.map(|h| h.join(".config")));
        dirs.extend(config.map(|c| c.join("REAPER").join("Effects")));
    }
    dirs.retain(|d| d.is_dir());
    dirs
}

/// Whether `path` is an LV2 bundle that declares a plugin: a `.lv2` directory whose
/// `manifest.ttl` mentions `Plugin` (every plugin is declared there with `a lv2:Plugin`; LV2's
/// specification and preset-only bundles aren't, so they never cost a sandbox).
pub fn is_lv2_plugin_bundle(path: &Path) -> bool {
    PluginFormat::of_path(path) == Some(PluginFormat::Lv2)
        && path.is_dir()
        && std::fs::read_to_string(path.join("manifest.ttl")).is_ok_and(|t| t.contains("Plugin"))
}

/// Whether `path` names an LV2 bundle (`.lv2`, any case) directory.
pub(crate) fn is_lv2_dir(path: &Path) -> bool {
    PluginFormat::of_path(path) == Some(PluginFormat::Lv2) && path.is_dir()
}

/// Whether `path` names a VST3 plugin (`.vst3`, any case).
pub(crate) fn is_vst3(path: &Path) -> bool {
    PluginFormat::of_path(path) == Some(PluginFormat::Vst3)
}

fn walk(dir: &Path, depth: usize, format: PluginFormat, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        // `metadata` follows symlinks (plugins are often linked into ~/.clap or ~/.vst3).
        let Ok(meta) = std::fs::metadata(&path) else {
            continue;
        };
        // Only a JSFX walk reads extensionless files' headers (T-808).
        let here = if format == PluginFormat::Jsfx {
            PluginFormat::of_path(&path)
        } else {
            PluginFormat::of_extension(&path)
        };
        if here == Some(format) {
            let keep = match format {
                // A macOS bundle is a directory; elsewhere the `.clap` is the library file.
                PluginFormat::Clap => meta.is_file() || cfg!(target_os = "macos"),
                // A bundle directory (never searched inside), or a legacy single file.
                PluginFormat::Vst3 => true,
                // A bundle directory that declares a plugin (never searched inside).
                PluginFormat::Lv2 => is_lv2_plugin_bundle(&path),
                // A script file (`.jsfx`, or sniffed by `of_path`).
                PluginFormat::Jsfx => meta.is_file(),
            };
            if keep {
                out.push(path);
            }
        } else if meta.is_dir() && depth < MAX_DEPTH {
            walk(&path, depth + 1, format, out);
        }
    }
}

fn find_files(format: PluginFormat, dirs: &[PathBuf]) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for d in dirs {
        walk(d, 0, format, &mut out);
    }
    let mut seen = HashSet::new();
    out.retain(|p| seen.insert(std::fs::canonicalize(p).unwrap_or_else(|_| p.clone())));
    out.sort();
    out
}

/// Every `.clap` file below `dirs` (recursively), sorted, without duplicates.
pub fn find_clap_files(dirs: &[PathBuf]) -> Vec<PathBuf> {
    find_files(PluginFormat::Clap, dirs)
}

/// Every `.vst3` bundle below `dirs` (recursively, never inside a bundle), sorted, without
/// duplicates (T-806).
pub fn find_vst3_files(dirs: &[PathBuf]) -> Vec<PathBuf> {
    find_files(PluginFormat::Vst3, dirs)
}

/// Every LV2 plugin bundle below `dirs` (recursively, never inside a bundle), sorted, without
/// duplicates (T-807).
pub fn find_lv2_files(dirs: &[PathBuf]) -> Vec<PathBuf> {
    find_files(PluginFormat::Lv2, dirs)
}

/// Every JSFX script below `dirs` (recursively), sorted, without duplicates (T-808).
pub fn find_jsfx_files(dirs: &[PathBuf]) -> Vec<PathBuf> {
    find_files(PluginFormat::Jsfx, dirs)
}

/// [`find_clap_files`], applied to a list of priority **tiers** instead of one flat directory
/// list (H-29, ADR-008 Amendment 5's duplicate-id policy): every file below one tier's
/// directories is found and sorted (by path) before moving on to the next tier, so the returned
/// order is "everything in tier 0, then everything in tier 1, …", each tier internally in path
/// order. [`effect_specs`]'s "first occurrence of an id wins" then applies this priority
/// directly and identically everywhere it's used (start-up's cached load, a quick rescan, a full
/// rescan) — a duplicate id is resolved the same way regardless of which one runs.
///
/// Without duplicates across tiers, by canonical path: a file reachable from two tiers (a
/// symlink, or a custom folder that happens to coincide with a standard one) keeps the position
/// of its highest-priority (earliest) tier only.
pub fn find_clap_files_ranked(tiers: &[Vec<PathBuf>]) -> Vec<PathBuf> {
    find_files_ranked(PluginFormat::Clap, tiers)
}

/// [`find_clap_files_ranked`] for VST3 bundles (T-806: the same H-29 tiers and duplicate
/// policy).
pub fn find_vst3_files_ranked(tiers: &[Vec<PathBuf>]) -> Vec<PathBuf> {
    find_files_ranked(PluginFormat::Vst3, tiers)
}

/// [`find_clap_files_ranked`] for LV2 bundles (T-807: the same H-29 tiers and duplicate policy).
pub fn find_lv2_files_ranked(tiers: &[Vec<PathBuf>]) -> Vec<PathBuf> {
    find_files_ranked(PluginFormat::Lv2, tiers)
}

/// [`find_clap_files_ranked`] for JSFX scripts (T-808: the same H-29 tiers and duplicate policy).
pub fn find_jsfx_files_ranked(tiers: &[Vec<PathBuf>]) -> Vec<PathBuf> {
    find_files_ranked(PluginFormat::Jsfx, tiers)
}

fn find_files_ranked(format: PluginFormat, tiers: &[Vec<PathBuf>]) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for tier in tiers {
        for f in find_files(format, tier) {
            let key = std::fs::canonicalize(&f).unwrap_or_else(|_| f.clone());
            if seen.insert(key) {
                out.push(f);
            }
        }
    }
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

/// The file whose size and mtime (and, for the blocklist, bytes) key a plugin: the file itself,
/// or for a VST3 bundle directory its binary for this platform — else its `moduleinfo.json` —
/// since a directory's own mtime doesn't change when the binary inside it is replaced (T-806).
pub(crate) fn key_file(path: &Path) -> PathBuf {
    if is_lv2_dir(path) {
        // T-807: the manifest (the blocklist's content hash); the stamp covers every file.
        return path.join("manifest.ttl");
    }
    if is_vst3(path) && path.is_dir() {
        if let Some(binary) = binary_path(path) {
            return binary;
        }
        let info = moduleinfo_path(path);
        if info.is_file() {
            return info;
        }
    }
    path.to_path_buf()
}

/// An LV2 bundle's stamp (T-807): the total size and the newest mtime of the files in it (two
/// levels deep), so replacing its binary or any `.ttl` counts as a change.
fn bundle_stamp(dir: &Path) -> Option<Stamp> {
    fn visit(dir: &Path, depth: usize, size: &mut u64, newest: &mut Duration, count: &mut usize) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let Ok(meta) = std::fs::metadata(entry.path()) else {
                continue;
            };
            if meta.is_dir() {
                if depth < 2 {
                    visit(&entry.path(), depth + 1, size, newest, count);
                }
                continue;
            }
            *size += meta.len();
            *count += 1;
            if let Some(t) = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            {
                *newest = (*newest).max(t);
            }
        }
    }
    let (mut size, mut newest, mut count) = (0u64, Duration::ZERO, 0usize);
    visit(dir, 0, &mut size, &mut newest, &mut count);
    (count > 0).then_some(Stamp {
        size,
        mtime_s: newest.as_secs(),
        mtime_ns: newest.subsec_nanos(),
    })
}

pub(crate) fn stamp(path: &Path) -> Option<Stamp> {
    if is_lv2_dir(path) {
        return bundle_stamp(path);
    }
    let meta = std::fs::metadata(key_file(path)).ok()?;
    let t = meta.modified().ok()?.duration_since(UNIX_EPOCH).ok()?;
    Some(Stamp {
        size: meta.len(),
        mtime_s: t.as_secs(),
        mtime_ns: t.subsec_nanos(),
    })
}

fn clap_format() -> String {
    CLAP_FORMAT.to_owned()
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct CacheEntry {
    path: String,
    /// The backend (T-806; entries written before it are CLAP's).
    #[serde(default = "clap_format")]
    format: String,
    #[serde(flatten)]
    stamp: Stamp,
    plugins: Vec<ScannedPlugin>,
}

fn format_name(path: &Path) -> String {
    format_of_path(path).unwrap_or(CLAP_FORMAT).to_owned()
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

/// Replaces the cache's entries of `formats` with `entries`, keeping every other format's
/// (T-806: one cache file for every format; a scan of some formats never drops another's).
fn store_cache(cache: &Path, formats: &HashSet<String>, entries: Vec<CacheEntry>) {
    let mut all = read_cache(cache);
    all.retain(|e| !formats.contains(&e.format));
    all.extend(entries);
    write_cache(cache, all);
}

/// Records one file's scan result in the cache (T-809: an installed plugin is known at the next
/// start without another sandboxed scan), replacing any entry for the same path. Skipped when the
/// file can't be stat'ed.
pub(crate) fn cache_upsert(cache: &Path, file: &Path, plugins: &[ScannedPlugin]) {
    let Some(stamp) = stamp(file) else {
        return;
    };
    let key = file.to_string_lossy().into_owned();
    let mut entries = read_cache(cache);
    entries.retain(|e| e.path != key);
    entries.push(CacheEntry {
        path: key,
        format: format_name(file),
        stamp,
        plugins: plugins.to_vec(),
    });
    write_cache(cache, entries);
}

/// JSON with the relaxations the VST3 SDK's `moduleinfotool` output needs: `//` and `/* */`
/// comments and trailing commas before `}`/`]` are dropped; strings are kept verbatim.
fn relaxed_json(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    // Pass 1: comments out.
    let mut no_comments = String::with_capacity(text.len());
    let (mut i, mut in_str, mut escaped) = (0, false, false);
    while i < chars.len() {
        let c = chars[i];
        if in_str {
            no_comments.push(c);
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_str = false;
            }
            i += 1;
        } else if c == '"' {
            in_str = true;
            no_comments.push(c);
            i += 1;
        } else if c == '/' && chars.get(i + 1) == Some(&'/') {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
        } else if c == '/' && chars.get(i + 1) == Some(&'*') {
            i += 2;
            while i < chars.len() && !(chars[i] == '*' && chars.get(i + 1) == Some(&'/')) {
                i += 1;
            }
            i += 2;
        } else {
            no_comments.push(c);
            i += 1;
        }
    }
    // Pass 2: trailing commas out.
    let chars: Vec<char> = no_comments.chars().collect();
    let mut out = String::with_capacity(no_comments.len());
    let (mut in_str, mut escaped) = (false, false);
    for (i, &c) in chars.iter().enumerate() {
        if in_str {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_str = false;
            }
        } else if c == '"' {
            in_str = true;
        } else if c == ','
            && matches!(
                chars[i + 1..].iter().find(|x| !x.is_whitespace()),
                Some('}' | ']')
            )
        {
            continue;
        }
        out.push(c);
    }
    out
}

#[derive(Default, Deserialize)]
struct ModuleInfoFactory {
    #[serde(rename = "Vendor", default)]
    vendor: String,
    #[serde(rename = "URL", default)]
    url: String,
}

#[derive(Deserialize)]
struct ModuleInfoClass {
    #[serde(rename = "CID")]
    cid: String,
    #[serde(rename = "Category", default)]
    category: String,
    #[serde(rename = "Name", default)]
    name: String,
    #[serde(rename = "Vendor", default)]
    vendor: String,
    #[serde(rename = "Version", default)]
    version: String,
    #[serde(rename = "Sub Categories", default)]
    sub_categories: Vec<String>,
}

#[derive(Deserialize)]
struct ModuleInfo {
    #[serde(rename = "Factory Info", default)]
    factory: Option<ModuleInfoFactory>,
    #[serde(rename = "Classes", default)]
    classes: Vec<ModuleInfoClass>,
}

/// A VST3 bundle's audio processor classes from its `Contents/Resources/moduleinfo.json`,
/// **without loading any code** (ADR-008 §6; T-806): every `Audio Module Class` with a valid
/// class id, features from its sub-categories. Parameter and port counts stay 0 (unknown: only
/// instantiating reveals them). `None` — not a bundle, no binary for this platform, no or an
/// unreadable `moduleinfo.json` — means the caller scans the bundle in a sandbox instead.
pub fn read_moduleinfo(bundle: &Path) -> Option<Vec<ScannedPlugin>> {
    if !is_vst3(bundle) || !bundle.is_dir() || binary_path(bundle).is_none() {
        return None;
    }
    let text = std::fs::read_to_string(moduleinfo_path(bundle)).ok()?;
    let info: ModuleInfo = serde_json::from_str(&relaxed_json(&text)).ok()?;
    let factory = info.factory.unwrap_or_default();
    let url = (!factory.url.trim().is_empty()).then(|| factory.url.trim().to_owned());
    Some(
        info.classes
            .into_iter()
            .filter(|c| c.category == AUDIO_MODULE_CLASS)
            .filter_map(|c| {
                let id = c.cid.trim().to_ascii_uppercase();
                if id.len() != 32 || !id.bytes().all(|b| b.is_ascii_hexdigit()) {
                    return None;
                }
                let name = c.name.trim();
                let vendor = c.vendor.trim();
                Some(ScannedPlugin {
                    name: if name.is_empty() {
                        id.clone()
                    } else {
                        name.to_owned()
                    },
                    id,
                    vendor: if vendor.is_empty() {
                        factory.vendor.trim().to_owned()
                    } else {
                        vendor.to_owned()
                    },
                    version: c.version.trim().to_owned(),
                    description: String::new(),
                    url: url.clone(),
                    features: vox_sandbox_ipc::vst3::features(&c.sub_categories.join("|")),
                    ..ScannedPlugin::default()
                })
            })
            .collect(),
    )
}

/// Drops `file`'s entry from the scan cache (H-29 "Uninstall…"): the file is gone, so a later
/// scan starts from scratch on it rather than ever trusting a stale hit for a plugin that no
/// longer exists. A no-op if it had no entry.
pub(crate) fn cache_remove(cache: &Path, file: &Path) {
    let key = file.to_string_lossy().into_owned();
    let mut entries = read_cache(cache);
    let before = entries.len();
    entries.retain(|e| e.path != key);
    if entries.len() != before {
        write_cache(cache, entries);
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

/// Scans one file in a `powervoice-sandbox --scan` process (killed after `timeout`); a VST3
/// bundle with `moduleinfo.json` is read from it instead, without a process (T-806).
pub fn scan_file(
    binary: &Path,
    path: &Path,
    timeout: Duration,
) -> Result<Vec<ScannedPlugin>, String> {
    scan_any(binary, path, timeout)
        .map(|(plugins, _)| plugins)
        .map_err(|e| e.message)
}

/// One file's plugins, and whether they came from `moduleinfo.json` (no sandbox).
fn scan_any(
    binary: &Path,
    path: &Path,
    timeout: Duration,
) -> Result<(Vec<ScannedPlugin>, bool), ScanFileError> {
    let format = PluginFormat::of_path(path).unwrap_or(PluginFormat::Clap);
    if format == PluginFormat::Vst3
        && let Some(plugins) = read_moduleinfo(path)
    {
        return Ok((plugins, true));
    }
    scan_file_inner(binary, path, timeout, format).map(|plugins| (plugins, false))
}

/// Scans one file like [`scan_file`], keeping whether the failure was a crash, a timeout or
/// anything else (T-809's "Install module…" blocklists only the first two, ADR-008 §5). Public
/// for [`crate::PluginCatalog::install_with`] callers that pick their own timeout (T-808 tests).
pub fn scan_one(
    binary: &Path,
    path: &Path,
    timeout: Duration,
) -> Result<Vec<ScannedPlugin>, ScanFailure> {
    scan_any(binary, path, timeout)
        .map(|(plugins, _)| plugins)
        .map_err(|e| ScanFailure {
            path: path.to_path_buf(),
            message: e.message,
            kind: e.kind,
        })
}

fn scan_file_inner(
    binary: &Path,
    path: &Path,
    timeout: Duration,
    format: PluginFormat,
) -> Result<Vec<ScannedPlugin>, ScanFileError> {
    let mut child = Command::new(binary)
        .arg("--scan")
        .arg(path)
        .arg("--format")
        .arg(format.name())
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
        .name("plugin-scan-rx".into())
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
/// the cache and blocklist. Files of any format (T-806: the format follows the extension).
pub fn scan_clap_files(files: &[PathBuf], options: &ScanOptions) -> ScanOutcome {
    scan_plugin_files_with(files, options, |_, _, _| {})
}

/// [`scan_clap_files`] under its format-neutral name (T-806).
pub fn scan_plugin_files(files: &[PathBuf], options: &ScanOptions) -> ScanOutcome {
    scan_plugin_files_with(files, options, |_, _, _| {})
}

/// [`scan_plugin_files_with`] (the name T-804 gave it).
pub fn scan_clap_files_with(
    files: &[PathBuf],
    options: &ScanOptions,
    on_progress: impl Fn(usize, usize, &Path) + Sync,
) -> ScanOutcome {
    scan_plugin_files_with(files, options, on_progress)
}

/// [`scan_clap_files`], calling `on_progress(done, total, current_file)` once per **live**
/// (not-blocklisted) file as it's accounted for (T-804 item 1: `plugin_scan_progress`) — cache
/// hits are reported immediately (before any sandbox runs), then sandboxed scans as they finish.
/// `done` is a plain 1, 2, 3, … counter (a `fetch_add` result), so progress is always reported in
/// increasing order even though the underlying scans run in parallel and can finish in any file
/// order.
///
/// A VST3 bundle with `moduleinfo.json` is indexed from it (no sandbox, counted in `indexed`).
pub fn scan_plugin_files_with(
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
    let results: Arc<Mutex<Vec<(usize, FileResult, bool)>>> = Arc::new(Mutex::new(Vec::new()));
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
                    let r = scan_any(&options.binary, &live[i], options.timeout);
                    report(&live[i]);
                    let (r, indexed) = match r {
                        Ok((plugins, indexed)) => (Ok(plugins), indexed),
                        Err(e) => (Err(e), false),
                    };
                    results
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .push((i, r, indexed));
                }
            });
        }
    });
    let results = std::mem::take(&mut *results.lock().unwrap_or_else(PoisonError::into_inner));
    for (i, r, indexed) in results {
        outcome.indexed += usize::from(indexed);
        slots[i].1 = Some(r);
    }
    outcome.scanned -= outcome.indexed;
    let mut entries = Vec::new();
    for (file, (st, result)) in live.iter().zip(slots) {
        match result {
            Some(Ok(plugins)) => {
                if let Some(stamp) = st {
                    entries.push(CacheEntry {
                        path: file.to_string_lossy().into_owned(),
                        format: format_name(file),
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
        let formats: HashSet<String> = files.iter().map(|f| format_name(f)).collect();
        store_cache(cache, &formats, entries);
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
    effect_specs(&cached_outcome(files, cache, blocklist))
}

/// [`cached_effect_specs`]'s underlying outcome: every plugin the cache knows for `files`
/// (current stamp, not blocklisted), with its full scan data (ports, parameter count — T-809's
/// plugin manager shows them). Never spawns a sandbox.
pub fn cached_outcome(
    files: &[PathBuf],
    cache: Option<&Path>,
    blocklist: Option<&Path>,
) -> ScanOutcome {
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
    outcome
}

/// Whether a scanned plugin is an audio effect the rack can host (no instruments or note
/// effects).
pub fn is_effect(p: &ScannedPlugin) -> bool {
    let has = |f: &str| p.features.iter().any(|x| x == f);
    has(features::AUDIO_EFFECT) && !has("instrument") && !has("note-effect")
}

/// An audio effect that lost a duplicate-id contest to an earlier file (H-29, ADR-008 Amendment
/// 5): it scanned fine, but another file already provides this id, so it isn't registered. The
/// plugin manager shows it with a "Shadowed by …" note instead of letting it vanish silently.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShadowedPlugin {
    /// The file that lost.
    pub path: PathBuf,
    /// What it scanned as (id, name, vendor, …) — everything the manager needs to show a row for
    /// it, since it's never registered.
    pub plugin: ScannedPlugin,
    /// The file whose same-id plugin actually won (is registered instead).
    pub shadowed_by: PathBuf,
}

/// [`effect_specs`], plus every audio effect that lost a duplicate-id contest to an earlier file
/// in `outcome.plugins`' order (H-29, ADR-008 Amendment 5: "first occurrence wins" — the caller
/// is responsible for that order expressing the actual priority, e.g. [`find_clap_files_ranked`]'s
/// tiers). A plugin can only shadow, or be shadowed by, another from a *different* file: two
/// entries the same file reports under one id can't happen (a CLAP file has one plugin per id).
pub fn effect_specs_with_shadows(outcome: &ScanOutcome) -> (Vec<SandboxSpec>, Vec<ShadowedPlugin>) {
    let mut winners: std::collections::HashMap<String, PathBuf> = std::collections::HashMap::new();
    let mut specs = Vec::new();
    let mut shadowed = Vec::new();
    for f in outcome.plugins.iter().filter(|f| is_effect(&f.plugin)) {
        let spec = spec_for(&f.path, &f.plugin);
        match winners.get(&spec.descriptor.id) {
            Some(winner) => shadowed.push(ShadowedPlugin {
                path: f.path.clone(),
                plugin: f.plugin.clone(),
                shadowed_by: winner.clone(),
            }),
            None => {
                winners.insert(spec.descriptor.id.clone(), f.path.clone());
                specs.push(spec);
            }
        }
    }
    (specs, shadowed)
}

/// The spec of `plugin` found in `path`, for the path's format (T-806: `vst3:` for a `.vst3`
/// bundle, `clap:` otherwise).
pub fn spec_for(path: &Path, plugin: &ScannedPlugin) -> SandboxSpec {
    match PluginFormat::of_path(path) {
        Some(PluginFormat::Vst3) => vst3_spec(path, plugin),
        Some(PluginFormat::Lv2) => lv2_spec(path, plugin),
        Some(PluginFormat::Jsfx) => jsfx_spec(path, plugin),
        _ => clap_spec(path, plugin),
    }
}

/// The specs of every audio effect in `outcome`; the first file wins for a duplicate id. See
/// [`effect_specs_with_shadows`] for what happens to the losers.
pub fn effect_specs(outcome: &ScanOutcome) -> Vec<SandboxSpec> {
    effect_specs_with_shadows(outcome).0
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
pub(crate) mod tests {
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

    /// H-29: the loser of a duplicate id is reported, not just dropped — with a pointer to the
    /// file that won.
    #[test]
    fn a_duplicate_id_is_reported_as_shadowed_by_the_winner() {
        let found = |path: &str, p| FoundPlugin {
            path: PathBuf::from(path),
            plugin: p,
        };
        let outcome = ScanOutcome {
            plugins: vec![
                found("/a.clap", plugin("com.x.eq", &["audio-effect"])),
                found("/b.clap", plugin("com.x.eq", &["audio-effect"])),
                found("/c.clap", plugin("com.x.eq", &["audio-effect"])),
                found("/b.clap", plugin("com.x.solo", &["audio-effect"])),
            ],
            ..ScanOutcome::default()
        };
        let (specs, shadowed) = effect_specs_with_shadows(&outcome);
        assert_eq!(specs.len(), 2, "com.x.eq once, com.x.solo once");
        assert!(specs.iter().any(|s| s.descriptor.id == "clap:com.x.eq"));
        assert!(specs.iter().any(|s| s.descriptor.id == "clap:com.x.solo"));
        assert_eq!(
            shadowed,
            vec![
                ShadowedPlugin {
                    path: PathBuf::from("/b.clap"),
                    plugin: plugin("com.x.eq", &["audio-effect"]),
                    shadowed_by: PathBuf::from("/a.clap"),
                },
                ShadowedPlugin {
                    path: PathBuf::from("/c.clap"),
                    plugin: plugin("com.x.eq", &["audio-effect"]),
                    shadowed_by: PathBuf::from("/a.clap"),
                },
            ]
        );
        // effect_specs (the plain version) is unaffected — same winners, no shadow info.
        let ids = |v: &[SandboxSpec]| {
            v.iter()
                .map(|s| s.descriptor.id.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(ids(&effect_specs(&outcome)), ids(&specs));
    }

    fn touch(dir: &Path, name: &str) -> PathBuf {
        let f = dir.join(name);
        std::fs::write(&f, b"x").unwrap();
        f
    }

    /// H-29's duplicate-id policy applies at the file-finding stage: a tier's files all come
    /// before the next tier's, and (since `effect_specs`'s dedup is "first occurrence wins")
    /// combined with a scan this is "the user's install folder beats the other standard paths,
    /// which beat custom folders; ties go to path order" — exactly by putting the install
    /// folder, then the standard paths, then the custom folders, in that tier order.
    #[test]
    fn ranked_file_finding_orders_tiers_before_paths_within_a_tier() {
        let dir = temp_dir("ranked");
        let install = dir.join("install");
        let standard_a = dir.join("standard-a");
        let standard_b = dir.join("standard-b");
        let custom = dir.join("custom");
        for d in [&install, &standard_a, &standard_b, &custom] {
            std::fs::create_dir_all(d).unwrap();
        }
        // Two files in the same (standard) tier: path order breaks the tie between them.
        let zeta = touch(&standard_b, "zeta.clap");
        let alpha = touch(&standard_a, "alpha.clap");
        let installed = touch(&install, "acme.clap");
        let custom_file = touch(&custom, "custom.clap");

        let tiers = vec![
            vec![install.clone()],
            vec![standard_a.clone(), standard_b.clone()],
            vec![custom.clone()],
        ];
        let files = find_clap_files_ranked(&tiers);
        assert_eq!(files, vec![installed, alpha, zeta, custom_file]);

        std::fs::remove_dir_all(&dir).ok();
    }

    /// A file reachable from two tiers (a symlink into a lower-priority folder, say) is only
    /// ever listed once, at its highest-priority position.
    #[cfg(unix)]
    #[test]
    fn ranked_file_finding_deduplicates_across_tiers() {
        let dir = temp_dir("ranked-dedup");
        let install = dir.join("install");
        let standard = dir.join("standard");
        std::fs::create_dir_all(&install).unwrap();
        std::fs::create_dir_all(&standard).unwrap();
        let real = touch(&install, "acme.clap");
        std::os::unix::fs::symlink(&real, standard.join("acme.clap")).unwrap();

        let files = find_clap_files_ranked(&[vec![install.clone()], vec![standard.clone()]]);
        assert_eq!(files, vec![real]);

        std::fs::remove_dir_all(&dir).ok();
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

    // --- T-806: VST3 --------------------------------------------------------------------------

    /// A VST3 bundle `<dir>/<name>.vst3` with a binary for this platform (and `moduleinfo`).
    pub(crate) fn vst3_bundle(dir: &Path, name: &str, moduleinfo: Option<&str>) -> PathBuf {
        let bundle = dir.join(format!("{name}.vst3"));
        let arch = bundle
            .join("Contents")
            .join(vox_sandbox_ipc::vst3::arch_folder());
        std::fs::create_dir_all(&arch).unwrap();
        let file = if cfg!(target_os = "macos") {
            name.to_owned()
        } else if cfg!(windows) {
            format!("{name}.vst3")
        } else {
            format!("{name}.so")
        };
        std::fs::write(arch.join(file), b"binary").unwrap();
        if let Some(text) = moduleinfo {
            let res = bundle.join("Contents").join("Resources");
            std::fs::create_dir_all(&res).unwrap();
            std::fs::write(res.join("moduleinfo.json"), text).unwrap();
        }
        bundle
    }

    /// What the SDK's `moduleinfotool` writes (JSON5-style trailing commas), plus comments.
    pub(crate) const MODULEINFO: &str = r#"{
  // Written by moduleinfotool
  "Name": "Acme",
  "Version": "2.0.0",
  "Factory Info": {
    "Vendor": "Acme Audio",
    "URL": "https://acme.example",
    "E-Mail": "mailto:info@acme.example",
    "Flags": { "Unicode": true, "Classes Discardable": false, },
  },
  "Compatibility": [ ],
  "Classes": [
    {
      "CID": "84e8de5f92554f5396fae4133c935a18",
      "Category": "Audio Module Class",
      "Name": "Acme De-esser",
      "Vendor": "",
      "Version": "2.0.0",
      "SDKVersion": "VST 3.8.0",
      "Sub Categories": [ "Fx", "Dynamics", ],
      "Class Flags": 1,
      "Cardinality": 2147483647,
      "Snapshots": [ ],
    },
    /* the controller class is not a plugin */
    {
      "CID": "D39D5B65D7AF42FA843F4AC841EB04F0",
      "Category": "Component Controller Class",
      "Name": "Acme De-esser Controller",
      "Sub Categories": [ ],
    },
    {
      "CID": "0000000000000000000000000000ABCD",
      "Category": "Audio Module Class",
      "Name": "Acme Synth, \"ten\" // voices",
      "Vendor": "Acme Instruments",
      "Version": "1.0",
      "Sub Categories": [ "Instrument", "Synth", ],
    },
    { "CID": "not-a-cid", "Category": "Audio Module Class", "Name": "Broken", },
  ],
}"#;

    #[test]
    fn relaxed_json_drops_comments_and_trailing_commas_but_keeps_strings() {
        let v: serde_json::Value = serde_json::from_str(&relaxed_json(
            "{ // c\n \"a\": \"x, ] // /* kept\", /* c */ \"b\": [1, 2, ], }",
        ))
        .unwrap();
        assert_eq!(v["a"], "x, ] // /* kept");
        assert_eq!(v["b"], serde_json::json!([1, 2]));
    }

    #[test]
    fn moduleinfo_lists_audio_classes_without_loading_the_bundle() {
        let dir = temp_dir("moduleinfo");
        let bundle = vst3_bundle(&dir, "Acme", Some(MODULEINFO));
        let plugins = read_moduleinfo(&bundle).expect("indexed");
        assert_eq!(plugins.len(), 2, "controller and broken classes skipped");
        let fx = &plugins[0];
        assert_eq!(fx.id, "84E8DE5F92554F5396FAE4133C935A18");
        assert_eq!(
            (fx.name.as_str(), fx.vendor.as_str(), fx.version.as_str()),
            ("Acme De-esser", "Acme Audio", "2.0.0")
        );
        assert_eq!(fx.url.as_deref(), Some("https://acme.example"));
        assert_eq!(fx.features, ["audio-effect", "compressor"]);
        assert!(is_effect(fx));
        assert_eq!(fx.param_count, 0, "unknown without instantiating");
        let synth = &plugins[1];
        assert_eq!(synth.name, "Acme Synth, \"ten\" // voices");
        assert!(!is_effect(synth));
        let spec = spec_for(&bundle, fx);
        assert_eq!(spec.descriptor.id, "vst3:84E8DE5F92554F5396FAE4133C935A18");
        assert_eq!(spec.format, "vst3");
        assert_eq!(spec.path(), Some(bundle.clone()));

        // No moduleinfo, no binary for this platform, or not a bundle: a sandbox scan instead.
        assert!(read_moduleinfo(&vst3_bundle(&dir, "Plain", None)).is_none());
        let no_binary = dir.join("NoBinary.vst3");
        std::fs::create_dir_all(no_binary.join("Contents").join("Resources")).unwrap();
        std::fs::write(moduleinfo_path(&no_binary), MODULEINFO).unwrap();
        assert!(read_moduleinfo(&no_binary).is_none());
        let broken = vst3_bundle(&dir, "Broken", Some("{ nope"));
        assert!(read_moduleinfo(&broken).is_none());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn vst3_bundles_are_found_but_never_searched_inside() {
        let dir = temp_dir("vst3-find");
        let a = vst3_bundle(&dir.join("vendor"), "Alpha", None);
        // A bundle nested inside a bundle is not a plugin of its own.
        vst3_bundle(&a.join("Contents").join("Resources"), "Inner", None);
        let single = dir.join("Legacy.vst3");
        std::fs::write(&single, b"dll").unwrap();
        std::fs::write(dir.join("readme.txt"), b"x").unwrap();
        assert_eq!(
            find_vst3_files(std::slice::from_ref(&dir)),
            vec![single.clone(), a.clone()]
        );
        assert!(find_clap_files(std::slice::from_ref(&dir)).is_empty());
        assert_eq!(PluginFormat::of_path(&a), Some(PluginFormat::Vst3));
        assert_eq!(format_of_path(Path::new("/x/y.CLAP")), Some("clap"));
        assert_eq!(format_of_path(Path::new("/x/y.so")), None);
        // Ranked like CLAP (H-29 tiers).
        let install = dir.join("install");
        let installed = vst3_bundle(&install, "Alpha", None);
        let ranked = find_vst3_files_ranked(&[vec![install.clone()], vec![dir.join("vendor")]]);
        assert_eq!(ranked, vec![installed, a]);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_vst3_bundles_stamp_follows_its_binary() {
        let dir = temp_dir("vst3-stamp");
        let bundle = vst3_bundle(&dir, "Stamped", None);
        let binary = binary_path(&bundle).unwrap();
        assert_eq!(key_file(&bundle), binary);
        let before = stamp(&bundle).unwrap();
        std::fs::write(&binary, b"a longer, rebuilt binary").unwrap();
        assert_ne!(
            stamp(&bundle).unwrap(),
            before,
            "a rebuilt binary is a changed plugin"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_cache_keeps_other_formats_entries() {
        let dir = temp_dir("cache-formats");
        let cache = dir.join("cache.json");
        let clap = dir.join("a.clap");
        std::fs::write(&clap, b"x").unwrap();
        let vst3 = vst3_bundle(&dir, "B", None);
        cache_upsert(&cache, &clap, &[plugin("com.x.a", &["audio-effect"])]);
        cache_upsert(
            &cache,
            &vst3,
            &[plugin(
                "00000000000000000000000000000001",
                &["audio-effect"],
            )],
        );
        let formats = |e: &[CacheEntry]| {
            let mut f: Vec<String> = e.iter().map(|e| e.format.clone()).collect();
            f.sort();
            f
        };
        assert_eq!(formats(&read_cache(&cache)), ["clap", "vst3"]);
        // A scan of only CLAP files replaces CLAP entries and keeps the VST3 ones.
        store_cache(&cache, &HashSet::from(["clap".to_owned()]), Vec::new());
        assert_eq!(formats(&read_cache(&cache)), ["vst3"]);
        // An entry written before T-806 (no `format`) is CLAP's.
        let st = stamp(&clap).unwrap();
        let old = serde_json::json!({"version": CACHE_VERSION, "entries": [{
            "path": clap.to_string_lossy(), "size": st.size, "mtime_s": st.mtime_s,
            "mtime_ns": st.mtime_ns, "plugins": []}]});
        std::fs::write(&cache, serde_json::to_vec(&old).unwrap()).unwrap();
        assert_eq!(formats(&read_cache(&cache)), ["clap"]);
        std::fs::remove_dir_all(&dir).ok();
    }

    // --- T-807: LV2 ------------------------------------------------------------------------

    /// An LV2 bundle `<dir>/<name>.lv2` whose manifest declares a plugin (or, `plugin: false`,
    /// only a specification).
    pub(crate) fn lv2_bundle(dir: &Path, name: &str, plugin: bool) -> PathBuf {
        let bundle = dir.join(format!("{name}.lv2"));
        std::fs::create_dir_all(&bundle).unwrap();
        let manifest = if plugin {
            "@prefix lv2: <http://lv2plug.in/ns/lv2core#> .\n<urn:acme:eq> a lv2:Plugin ; lv2:binary <eq.so> .\n"
        } else {
            "@prefix lv2: <http://lv2plug.in/ns/lv2core#> .\n<urn:acme:ns> a lv2:Specification .\n"
        };
        std::fs::write(bundle.join("manifest.ttl"), manifest).unwrap();
        std::fs::write(bundle.join("eq.so"), b"binary").unwrap();
        bundle
    }

    #[test]
    fn lv2_plugin_bundles_are_found_but_never_searched_inside() {
        let dir = temp_dir("lv2-find");
        let a = lv2_bundle(&dir.join("vendor"), "Alpha", true);
        lv2_bundle(&a, "Inner", true);
        lv2_bundle(&dir, "spec", false);
        std::fs::create_dir_all(dir.join("empty.lv2")).unwrap();
        std::fs::write(dir.join("file.lv2"), b"not a bundle").unwrap();
        assert_eq!(
            find_lv2_files(std::slice::from_ref(&dir)),
            vec![a.clone()],
            "specification bundles, folders without a manifest and files are skipped"
        );
        assert_eq!(PluginFormat::of_path(&a), Some(PluginFormat::Lv2));
        assert_eq!(format_of_path(&a), Some("lv2"));
        let spec = spec_for(&a, &plugin("urn:acme:eq", &["audio-effect"]));
        assert_eq!(
            (spec.descriptor.id.as_str(), spec.format.as_str()),
            ("lv2:urn:acme:eq", "lv2")
        );
        assert_eq!(spec.path(), Some(a.clone()));
        // H-29 tiers: the install folder's copy comes first.
        let install = dir.join("home").join(".lv2");
        let installed = lv2_bundle(&install, "Alpha", true);
        let ranked = find_lv2_files_ranked(&[vec![install.clone()], vec![dir.join("vendor")]]);
        assert_eq!(ranked, vec![installed, a]);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn an_lv2_bundles_stamp_covers_its_files_and_its_key_is_the_manifest() {
        let dir = temp_dir("lv2-stamp");
        let bundle = lv2_bundle(&dir, "Stamped", true);
        assert_eq!(key_file(&bundle), bundle.join("manifest.ttl"));
        let before = stamp(&bundle).unwrap();
        std::fs::write(bundle.join("eq.so"), b"a longer, rebuilt binary").unwrap();
        let rebuilt = stamp(&bundle).unwrap();
        assert_ne!(rebuilt, before, "a rebuilt binary is a changed plugin");
        std::fs::create_dir_all(bundle.join("presets")).unwrap();
        std::fs::write(bundle.join("presets").join("p.ttl"), b"x").unwrap();
        assert_ne!(
            stamp(&bundle).unwrap(),
            rebuilt,
            "a file one level down counts"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn search_paths_include_the_standard_lv2_folders() {
        let dirs = lv2_search_paths();
        #[cfg(all(unix, not(target_os = "macos")))]
        {
            assert!(dirs.contains(&PathBuf::from("/usr/lib/lv2")));
            assert!(dirs.contains(&PathBuf::from("/usr/local/lib/lv2")));
            if let Some(home) = std::env::var_os("HOME") {
                assert!(dirs.contains(&PathBuf::from(home).join(".lv2")));
            }
        }
        #[cfg(windows)]
        assert!(dirs.is_empty());
        let mut unique = dirs.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), dirs.len(), "no duplicates");
    }

    #[test]
    fn search_paths_include_the_standard_vst3_folders() {
        let dirs = vst3_search_paths();
        #[cfg(all(unix, not(target_os = "macos")))]
        {
            assert!(dirs.contains(&PathBuf::from("/usr/lib/vst3")));
            assert!(dirs.contains(&PathBuf::from("/usr/local/lib/vst3")));
        }
        if let Some(home) = std::env::var_os("HOME")
            && cfg!(all(unix, not(target_os = "macos")))
        {
            assert!(dirs.contains(&PathBuf::from(home).join(".vst3")));
        }
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

    // --- H-34: `clap-scan.json` → `plugin-scan.json` -------------------------------------------

    /// A leftover cache from before the rename is picked up: its content survives under the new
    /// name, and the old file is gone (so it's never read again).
    #[test]
    fn migrate_cache_file_name_renames_a_leftover_legacy_file() {
        let dir = temp_dir("migrate-hit");
        let old_path = dir.join(LEGACY_CACHE_FILE_NAME);
        let new_path = dir.join(CACHE_FILE_NAME);
        std::fs::write(&old_path, b"{\"version\":2,\"entries\":[]}").unwrap();

        migrate_cache_file_name(&new_path);

        assert!(new_path.is_file(), "new cache file should exist");
        assert!(!old_path.exists(), "legacy file should be gone");
        assert_eq!(
            std::fs::read_to_string(&new_path).unwrap(),
            "{\"version\":2,\"entries\":[]}"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    /// No legacy file: a first run (or an already-migrated one with no cache at all) is a no-op,
    /// not an error.
    #[test]
    fn migrate_cache_file_name_is_a_no_op_without_a_legacy_file() {
        let dir = temp_dir("migrate-miss");
        let new_path = dir.join(CACHE_FILE_NAME);

        migrate_cache_file_name(&new_path);

        assert!(!new_path.exists());
        assert!(!dir.join(LEGACY_CACHE_FILE_NAME).exists());

        std::fs::remove_dir_all(&dir).ok();
    }

    /// A cache already sitting under the new name always wins — even a leftover legacy file
    /// (which shouldn't happen in practice, but this keeps the migration one-shot and
    /// never-destructive) is left alone rather than overwriting real data.
    #[test]
    fn migrate_cache_file_name_never_overwrites_an_existing_new_file() {
        let dir = temp_dir("migrate-existing");
        let old_path = dir.join(LEGACY_CACHE_FILE_NAME);
        let new_path = dir.join(CACHE_FILE_NAME);
        std::fs::write(&old_path, b"old").unwrap();
        std::fs::write(&new_path, b"new").unwrap();

        migrate_cache_file_name(&new_path);

        assert_eq!(std::fs::read_to_string(&new_path).unwrap(), "new");
        assert_eq!(std::fs::read_to_string(&old_path).unwrap(), "old");

        std::fs::remove_dir_all(&dir).ok();
    }

    /// The migration also works end to end through [`scan_clap_files`]'s cache: entries written
    /// under the legacy name are found (and re-persisted under the new one) after migrating.
    #[test]
    fn a_migrated_cache_still_answers_scan_clap_files_from_cache() {
        let dir = temp_dir("migrate-e2e");
        let file = dir.join("plugin.clap");
        std::fs::write(&file, b"x").unwrap();
        let legacy_cache = dir.join(LEGACY_CACHE_FILE_NAME);
        let st = stamp(&file).unwrap();
        let seeded = serde_json::json!({
            "version": CACHE_VERSION,
            "entries": [{
                "path": file.to_string_lossy(),
                "format": "clap",
                "size": st.size,
                "mtime_s": st.mtime_s,
                "mtime_ns": st.mtime_ns,
                "plugins": [],
            }],
        });
        std::fs::write(&legacy_cache, serde_json::to_vec(&seeded).unwrap()).unwrap();

        let new_cache = dir.join(CACHE_FILE_NAME);
        migrate_cache_file_name(&new_cache);
        assert!(new_cache.is_file());

        let mut options = ScanOptions::new(PathBuf::from("/nonexistent/powervoice-sandbox"));
        options.cache = Some(new_cache);
        let out = scan_clap_files(std::slice::from_ref(&file), &options);
        assert_eq!((out.cached, out.scanned), (1, 0), "{out:?}");

        std::fs::remove_dir_all(&dir).ok();
    }
}

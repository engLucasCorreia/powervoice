//! "Install module…" (T-809, ADR-006 §7 as amended by T-809): copy a plugin file the user picked
//! into the **per-user** CLAP folder, scan only that file, and roll back if it can't be used.
//! T-806: a `.vst3` bundle (a directory on every OS; Windows' legacy single file too) goes into
//! the per-user VST3 folder the same way — [`user_install_dir_for`] picks the folder by format:
//! `~/.vst3`, `~/Library/Audio/Plug-Ins/VST3` or `%LOCALAPPDATA%\Programs\Common\VST3`. A
//! path picked *inside* a bundle (a file dialog can't select a directory) stands for the bundle.
//! T-807: an `.lv2` bundle directory (unix) goes into the per-user LV2 folder — `~/.lv2`, or
//! `~/Library/Audio/Plug-Ins/LV2` on macOS — the same way; picking its `manifest.ttl` (or any
//! file inside it) stands for the bundle too.
//!
//! Where it goes — the standard per-user CLAP path (CLAP `entry.h`), already one of the folders
//! [`crate::scan::clap_search_paths`] searches, so a later rescan (and other CLAP hosts) find it:
//! - Linux/BSD: `~/.clap`;
//! - macOS: `~/Library/Audio/Plug-Ins/CLAP` (a `.clap` there is a bundle directory);
//! - Windows: `%LOCALAPPDATA%\Programs\Common\CLAP`.
//!
//! Never a system folder (`/usr/lib/clap`, `/Library/…`, `%COMMONPROGRAMFILES%`): no admin rights,
//! and the caller passes the destination, which `src-tauri` only ever gets from [`user_clap_dir`].
//!
//! The copy is staged next to the destination under a hidden name that isn't `*.clap` (so a scan
//! running at the same time never picks it up), then renamed into place. A file with the same
//! name is only replaced when the caller says so (the UI asks first); the old one is kept aside
//! until the new one has scanned, and restored if it doesn't. A scan that **crashes or times
//! out** blocklists the *picked* file (ADR-008 §5), so installing it again is refused until it
//! changes or the user unblocks it — the plugin manager lists it with the reason.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::blocklist::{BlockReason, Blocklist};
use crate::scan::{FailureKind, PluginFormat, ScanFailure, ScannedPlugin, is_effect};

/// The per-user CLAP folder for this OS, from the environment (`HOME`, or `LOCALAPPDATA` on
/// Windows). `None` when that variable is unset, empty or not absolute.
pub fn user_clap_dir() -> Option<PathBuf> {
    user_clap_dir_from(
        std::env::var_os("HOME").as_deref(),
        std::env::var_os("LOCALAPPDATA").as_deref(),
    )
}

fn absolute(value: Option<&OsStr>) -> Option<PathBuf> {
    value.map(PathBuf::from).filter(|p| p.is_absolute())
}

/// [`user_clap_dir`] for an explicit `HOME` / `LOCALAPPDATA` (tests use a temporary home).
#[cfg(windows)]
pub fn user_clap_dir_from(
    _home: Option<&OsStr>,
    local_app_data: Option<&OsStr>,
) -> Option<PathBuf> {
    absolute(local_app_data).map(|d| d.join("Programs").join("Common").join("CLAP"))
}

/// [`user_clap_dir`] for an explicit `HOME` / `LOCALAPPDATA` (tests use a temporary home).
#[cfg(target_os = "macos")]
pub fn user_clap_dir_from(
    home: Option<&OsStr>,
    _local_app_data: Option<&OsStr>,
) -> Option<PathBuf> {
    absolute(home).map(|h| {
        h.join("Library")
            .join("Audio")
            .join("Plug-Ins")
            .join("CLAP")
    })
}

/// [`user_clap_dir`] for an explicit `HOME` / `LOCALAPPDATA` (tests use a temporary home).
#[cfg(all(unix, not(target_os = "macos")))]
pub fn user_clap_dir_from(
    home: Option<&OsStr>,
    _local_app_data: Option<&OsStr>,
) -> Option<PathBuf> {
    absolute(home).map(|h| h.join(".clap"))
}

/// The per-user VST3 folder for this OS (T-806), from the environment like [`user_clap_dir`].
pub fn user_vst3_dir() -> Option<PathBuf> {
    user_vst3_dir_from(
        std::env::var_os("HOME").as_deref(),
        std::env::var_os("LOCALAPPDATA").as_deref(),
    )
}

/// [`user_vst3_dir`] for an explicit `HOME` / `LOCALAPPDATA` (tests use a temporary home).
pub fn user_vst3_dir_from(home: Option<&OsStr>, local_app_data: Option<&OsStr>) -> Option<PathBuf> {
    if cfg!(windows) {
        absolute(local_app_data).map(|d| d.join("Programs").join("Common").join("VST3"))
    } else if cfg!(target_os = "macos") {
        absolute(home).map(|h| {
            h.join("Library")
                .join("Audio")
                .join("Plug-Ins")
                .join("VST3")
        })
    } else {
        absolute(home).map(|h| h.join(".vst3"))
    }
}

/// The per-user LV2 folder for this OS (T-807; `None` on Windows, where LV2 isn't hosted), from
/// the environment like [`user_clap_dir`].
pub fn user_lv2_dir() -> Option<PathBuf> {
    user_lv2_dir_from(std::env::var_os("HOME").as_deref())
}

/// [`user_lv2_dir`] for an explicit `HOME` (tests use a temporary home).
pub fn user_lv2_dir_from(home: Option<&OsStr>) -> Option<PathBuf> {
    if cfg!(windows) {
        None
    } else if cfg!(target_os = "macos") {
        absolute(home).map(|h| h.join("Library").join("Audio").join("Plug-Ins").join("LV2"))
    } else {
        absolute(home).map(|h| h.join(".lv2"))
    }
}

/// The per-user folder a plugin of `path`'s format installs into (`None`: not a plugin path,
/// or no per-user folder in this environment).
pub fn user_install_dir_for(path: &Path) -> Option<PathBuf> {
    match PluginFormat::of_path(&plugin_root(path))? {
        PluginFormat::Clap => user_clap_dir(),
        PluginFormat::Vst3 => user_vst3_dir(),
        PluginFormat::Lv2 => user_lv2_dir(),
    }
}

/// `path`, or the `.vst3` / `.lv2` bundle directory it lies inside (a file dialog can only pick a
/// file of a Linux/Windows bundle, e.g. its binary, `moduleinfo.json` or `manifest.ttl`).
pub fn plugin_root(path: &Path) -> PathBuf {
    path.ancestors()
        .find(|a| {
            matches!(
                PluginFormat::of_path(a),
                Some(PluginFormat::Vst3 | PluginFormat::Lv2)
            ) && a.is_dir()
        })
        .unwrap_or(path)
        .to_path_buf()
}

/// Why an install didn't happen. Every variant leaves the destination as it was.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum InstallError {
    /// The picked file doesn't exist (or can't be read).
    #[error("the file doesn't exist")]
    NotFound,
    /// Not a `.clap` file (or, outside macOS, a directory), nor a `.vst3` bundle, nor (unix) an
    /// `.lv2` bundle.
    #[error("not a CLAP, VST3 or LV2 plugin")]
    NotAPlugin,
    /// The picked file already *is* the installed copy.
    #[error("this plugin is already installed here")]
    AlreadyInstalled,
    /// The picked file is blocklisted (a previous scan crashed or timed out, or the user blocked
    /// it); unblock it in the plugin manager first.
    #[error("the file is blocklisted: {}", .0.message())]
    Blocklisted(BlockReason),
    /// A file with the same name is already installed; ask, then call again with `replace`.
    #[error("a plugin with this name is already installed")]
    Collision {
        /// The installed file that would be replaced.
        target: PathBuf,
    },
    /// It scanned, but offers no audio effect the rack can host (an instrument, say).
    #[error("it contains no audio effects PowerVoice can use")]
    NoEffects,
    /// The sandboxed scan failed (`kind` says how; `blocklisted`: the picked file was
    /// blocklisted because the scan crashed or timed out).
    #[error("{message}")]
    ScanFailed {
        message: String,
        kind: FailureKind,
        blocklisted: bool,
    },
    /// The environment names no per-user plugin folder (`HOME` unset…).
    #[error("there is no per-user plugin folder")]
    NoInstallDir,
    /// Copying or renaming failed.
    #[error("{0}")]
    Io(String),
}

fn io(e: std::io::Error) -> InstallError {
    InstallError::Io(e.to_string())
}

/// A successful install.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Installed {
    /// Where it was copied to.
    pub target: PathBuf,
    /// Everything the file offers (the caller registers the audio effects among them).
    pub plugins: Vec<ScannedPlugin>,
    /// An older file with the same name was replaced.
    pub replaced: bool,
}

/// A hidden sibling of `name` in `dir` that a scan never picks up (doesn't end in `.clap`).
fn sibling(dir: &Path, name: &OsStr, tag: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    dir.join(format!(
        ".{}.pv-{tag}-{}-{nanos}",
        name.to_string_lossy(),
        std::process::id()
    ))
}

/// Removes a file, a symlink or a whole directory.
fn remove_any(path: &Path) -> std::io::Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(m) if m.is_dir() => std::fs::remove_dir_all(path),
        Ok(_) => std::fs::remove_file(path),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

/// Deletes `0` when dropped, unless disarmed (a staging copy on an error path).
struct Cleanup(Option<PathBuf>);

impl Drop for Cleanup {
    fn drop(&mut self) {
        if let Some(p) = self.0.take() {
            let _ = remove_any(&p);
        }
    }
}

/// Copies a directory tree (a macOS bundle); symlinks inside it stay symlinks on Unix.
pub(crate) fn copy_dir_all(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let src = entry.path();
        let dst = to.join(entry.file_name());
        let kind = entry.file_type()?;
        if kind.is_dir() {
            copy_dir_all(&src, &dst)?;
        } else if kind.is_symlink() {
            #[cfg(unix)]
            std::os::unix::fs::symlink(std::fs::read_link(&src)?, &dst)?;
            #[cfg(not(unix))]
            std::fs::copy(&src, &dst).map(|_| ())?;
        } else {
            std::fs::copy(&src, &dst)?;
        }
    }
    Ok(())
}

/// Copies a file (and syncs it) or a bundle directory.
fn copy_any(from: &Path, to: &Path, is_dir: bool) -> std::io::Result<()> {
    if is_dir {
        return copy_dir_all(from, to);
    }
    std::fs::copy(from, to)?;
    std::fs::File::open(to)?.sync_all()
}

fn same_file(a: &Path, b: &Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

/// Installs `source` into `dest_dir` (see the module docs). `scan` is called **once**, with the
/// installed path only — the rest of the plugin folders are never rescanned for an install
/// (production: one `powervoice-sandbox --scan` process). `blocklist` is checked for the picked
/// file first and records a crash/timeout of the scan against it.
pub fn install_file(
    source: &Path,
    dest_dir: &Path,
    replace: bool,
    blocklist: &mut Blocklist,
    scan: impl FnOnce(&Path) -> Result<Vec<ScannedPlugin>, ScanFailure>,
) -> Result<Installed, InstallError> {
    let source = &plugin_root(source);
    let meta = std::fs::metadata(source).map_err(|_| InstallError::NotFound)?;
    let is_dir = meta.is_dir();
    let supported = match PluginFormat::of_path(source) {
        // A `.clap` is a single file on Linux/Windows and a bundle directory on macOS.
        Some(PluginFormat::Clap) => !is_dir || cfg!(target_os = "macos"),
        // A `.vst3` is a bundle directory everywhere; Windows also has legacy single files.
        Some(PluginFormat::Vst3) => is_dir || cfg!(windows),
        // An `.lv2` is a bundle directory; the sandbox hosts LV2 on unix only.
        Some(PluginFormat::Lv2) => is_dir && cfg!(unix),
        None => false,
    };
    if !supported {
        return Err(InstallError::NotAPlugin);
    }
    let name = source.file_name().ok_or(InstallError::NotAPlugin)?;
    if let Some(info) = blocklist.check(source) {
        return Err(InstallError::Blocklisted(info.reason));
    }
    let target = dest_dir.join(name);
    if same_file(source, &target) {
        return Err(InstallError::AlreadyInstalled);
    }
    let exists = std::fs::symlink_metadata(&target).is_ok();
    if exists && !replace {
        return Err(InstallError::Collision { target });
    }

    std::fs::create_dir_all(dest_dir).map_err(io)?;
    let mut staging = Cleanup(Some(sibling(dest_dir, name, "staging")));
    let staged = staging.0.clone().unwrap_or_default();
    copy_any(source, &staged, is_dir).map_err(io)?;
    let backup = if exists {
        let b = sibling(dest_dir, name, "backup");
        std::fs::rename(&target, &b).map_err(io)?;
        Some(b)
    } else {
        None
    };
    if let Err(e) = std::fs::rename(&staged, &target) {
        if let Some(b) = &backup {
            let _ = std::fs::rename(b, &target);
        }
        return Err(io(e));
    }
    staging.0 = None;

    let rollback = || {
        let _ = remove_any(&target);
        if let Some(b) = &backup {
            let _ = std::fs::rename(b, &target);
        }
    };
    match scan(&target) {
        Ok(plugins) if plugins.iter().any(is_effect) => {
            if let Some(b) = &backup {
                let _ = remove_any(b);
            }
            Ok(Installed {
                target,
                plugins,
                replaced: exists,
            })
        }
        Ok(_) => {
            rollback();
            Err(InstallError::NoEffects)
        }
        Err(failure) => {
            rollback();
            let reason = match failure.kind {
                FailureKind::Crashed => Some(BlockReason::Crashed),
                FailureKind::TimedOut => Some(BlockReason::TimedOut),
                FailureKind::Other => None,
            };
            if let Some(reason) = reason {
                blocklist.block(source, reason);
            }
            Err(InstallError::ScanFailed {
                message: failure.message,
                kind: failure.kind,
                blocklisted: reason.is_some(),
            })
        }
    }
}

/// Why "Uninstall…" (H-29) didn't remove a file. Every variant leaves the file as it was.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum UninstallError {
    /// The file doesn't exist any more.
    #[error("the file doesn't exist")]
    NotFound,
    /// `path` isn't inside the per-user install folder. The plugin manager only offers
    /// "Uninstall…" for files there; anything found in a standard or custom folder offers
    /// "Block" instead — this is the server-side half of that rule (canonicalized, so a symlink
    /// can't be used to point "the installed file" somewhere else).
    #[error("this file isn't in the plugin folder PowerVoice installs into")]
    OutsideInstallFolder,
    /// The environment names no per-user plugin folder (`HOME` unset…) — nothing could ever have
    /// been installed, so nothing is inside it either.
    #[error("there is no per-user plugin folder")]
    NoInstallDir,
    /// Removing it failed.
    #[error("{0}")]
    Io(String),
}

/// Removes `target`, refusing anything outside `install_dir` (H-29; see the module docs and
/// [`UninstallError::OutsideInstallFolder`]). The caller ([`crate::catalog::PluginCatalog`])
/// drops the plugin from the registry and the scan cache; an open document that already uses it
/// keeps its slot — `vox_rack::Registry::resolve` falls back to the "Missing module" placeholder
/// once the id is gone from the registry, and the slot's own stored state is never touched by any
/// of this (ADR-006 §7 step 6).
pub fn uninstall_file(target: &Path, install_dir: &Path) -> Result<(), UninstallError> {
    let canon_target = std::fs::canonicalize(target).map_err(|_| UninstallError::NotFound)?;
    let canon_dir =
        std::fs::canonicalize(install_dir).map_err(|_| UninstallError::OutsideInstallFolder)?;
    if canon_target.parent() != Some(canon_dir.as_path()) {
        return Err(UninstallError::OutsideInstallFolder);
    }
    remove_any(target).map_err(io_uninstall)
}

fn io_uninstall(e: std::io::Error) -> UninstallError {
    UninstallError::Io(e.to_string())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::cell::RefCell;

    /// A temporary directory removed on drop (MEMORY: `/tmp` has a per-user quota — never leak).
    pub(crate) struct TempDir(pub PathBuf);

    impl TempDir {
        pub(crate) fn new(label: &str) -> Self {
            use std::sync::atomic::{AtomicU32, Ordering};
            static N: AtomicU32 = AtomicU32::new(0);
            let dir = std::env::temp_dir().join(format!(
                "pv-install-test-{label}-{}-{}",
                std::process::id(),
                N.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    pub(crate) fn effect(id: &str) -> ScannedPlugin {
        ScannedPlugin {
            id: id.into(),
            name: format!("{id} name"),
            vendor: "Acme".into(),
            version: "1.0".into(),
            features: vec!["audio-effect".into()],
            param_count: 7,
            main_input_channels: 1,
            main_output_channels: 2,
            ..ScannedPlugin::default()
        }
    }

    fn failure(kind: FailureKind) -> ScanFailure {
        ScanFailure {
            path: PathBuf::new(),
            message: "boom".into(),
            kind,
        }
    }

    /// A temp "home" with a picked plugin in `Downloads` and the per-user CLAP folder under it.
    struct Fixture {
        _root: TempDir,
        source: PathBuf,
        dest: PathBuf,
        blocklist: Blocklist,
    }

    fn fixture(label: &str, content: &[u8]) -> Fixture {
        let root = TempDir::new(label);
        let home = root.0.join("home");
        let downloads = home.join("Downloads");
        std::fs::create_dir_all(&downloads).unwrap();
        let source = downloads.join("acme-deesser.clap");
        std::fs::write(&source, content).unwrap();
        let dest = user_clap_dir_from(Some(home.as_os_str()), Some(home.as_os_str())).unwrap();
        let blocklist = Blocklist::load(Some(root.0.join("cache").join("blocklist.json")));
        Fixture {
            _root: root,
            source,
            dest,
            blocklist,
        }
    }

    fn visible_files(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .map(|rd| {
                rd.flatten()
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default();
        names.sort();
        names
    }

    #[test]
    fn the_install_folder_is_per_user_under_the_given_home() {
        let home = TempDir::new("home");
        let dir = user_clap_dir_from(Some(home.0.as_os_str()), Some(home.0.as_os_str())).unwrap();
        assert!(dir.starts_with(&home.0), "{dir:?}");
        #[cfg(all(unix, not(target_os = "macos")))]
        assert_eq!(dir, home.0.join(".clap"));
        // No system folder, ever — and no relative/empty home.
        assert!(!dir.starts_with("/usr") && !dir.starts_with("/Library"));
        assert_eq!(user_clap_dir_from(None, None), None);
        assert_eq!(
            user_clap_dir_from(Some(OsStr::new("")), Some(OsStr::new(""))),
            None
        );
        assert_eq!(
            user_clap_dir_from(Some(OsStr::new("relative")), Some(OsStr::new("relative"))),
            None
        );
    }

    #[test]
    fn installs_a_copy_and_scans_only_the_new_file() {
        let mut f = fixture("copy", b"plugin v1");
        let scanned = RefCell::new(Vec::new());
        let out = install_file(&f.source, &f.dest, false, &mut f.blocklist, |p| {
            scanned.borrow_mut().push(p.to_path_buf());
            Ok(vec![effect("com.acme.deesser")])
        })
        .unwrap();
        let target = f.dest.join("acme-deesser.clap");
        assert_eq!(out.target, target);
        assert!(!out.replaced);
        assert_eq!(out.plugins[0].id, "com.acme.deesser");
        assert_eq!(std::fs::read(&target).unwrap(), b"plugin v1");
        assert!(f.source.exists(), "the picked file is copied, not moved");
        assert_eq!(
            *scanned.borrow(),
            vec![target],
            "exactly one scan, of the new file"
        );
        assert_eq!(
            visible_files(&f.dest),
            vec!["acme-deesser.clap"],
            "no staging left behind"
        );
    }

    #[test]
    fn a_name_collision_needs_confirmation_then_replaces() {
        let mut f = fixture("collision", b"plugin v2");
        std::fs::create_dir_all(&f.dest).unwrap();
        let target = f.dest.join("acme-deesser.clap");
        std::fs::write(&target, b"plugin v1").unwrap();

        let err = install_file(&f.source, &f.dest, false, &mut f.blocklist, |_| {
            panic!("no scan before the user confirms")
        })
        .unwrap_err();
        assert_eq!(
            err,
            InstallError::Collision {
                target: target.clone()
            }
        );
        assert_eq!(std::fs::read(&target).unwrap(), b"plugin v1", "untouched");

        let out = install_file(&f.source, &f.dest, true, &mut f.blocklist, |_| {
            Ok(vec![effect("com.acme.deesser")])
        })
        .unwrap();
        assert!(out.replaced);
        assert_eq!(std::fs::read(&target).unwrap(), b"plugin v2");
        assert_eq!(
            visible_files(&f.dest),
            vec!["acme-deesser.clap"],
            "backup removed"
        );
    }

    #[test]
    fn a_failed_replacement_restores_the_previous_file() {
        let mut f = fixture("rollback", b"plugin v2 (broken)");
        std::fs::create_dir_all(&f.dest).unwrap();
        let target = f.dest.join("acme-deesser.clap");
        std::fs::write(&target, b"plugin v1").unwrap();
        let err = install_file(&f.source, &f.dest, true, &mut f.blocklist, |_| {
            Err(failure(FailureKind::Other))
        })
        .unwrap_err();
        assert_eq!(
            err,
            InstallError::ScanFailed {
                message: "boom".into(),
                kind: FailureKind::Other,
                blocklisted: false
            }
        );
        assert_eq!(std::fs::read(&target).unwrap(), b"plugin v1");
        assert_eq!(visible_files(&f.dest), vec!["acme-deesser.clap"]);
        assert!(
            f.blocklist.check(&f.source).is_none(),
            "a plain failure never blocklists"
        );
    }

    #[test]
    fn a_crashing_scan_rolls_back_and_blocklists_the_picked_file() {
        for (kind, reason) in [
            (FailureKind::Crashed, BlockReason::Crashed),
            (FailureKind::TimedOut, BlockReason::TimedOut),
        ] {
            let mut f = fixture("crash", b"crashes");
            let err = install_file(&f.source, &f.dest, false, &mut f.blocklist, |_| {
                Err(failure(kind))
            })
            .unwrap_err();
            assert!(
                matches!(
                    err,
                    InstallError::ScanFailed {
                        blocklisted: true,
                        ..
                    }
                ),
                "{err:?}"
            );
            assert!(visible_files(&f.dest).is_empty(), "nothing stays installed");
            assert_eq!(f.blocklist.check(&f.source).map(|i| i.reason), Some(reason));
            // Trying again is refused up front, without another scan.
            let again = install_file(&f.source, &f.dest, false, &mut f.blocklist, |_| {
                panic!("a blocklisted file is never scanned")
            })
            .unwrap_err();
            assert_eq!(again, InstallError::Blocklisted(reason));
        }
    }

    #[test]
    fn a_file_without_audio_effects_is_rolled_back() {
        let mut f = fixture("no-effects", b"synth");
        let err = install_file(&f.source, &f.dest, false, &mut f.blocklist, |_| {
            let mut synth = effect("com.acme.synth");
            synth.features = vec!["instrument".into()];
            Ok(vec![synth])
        })
        .unwrap_err();
        assert_eq!(err, InstallError::NoEffects);
        assert!(visible_files(&f.dest).is_empty());
    }

    #[test]
    fn rejects_missing_files_other_extensions_and_the_installed_copy_itself() {
        let mut f = fixture("reject", b"x");
        let never = |_: &Path| -> Result<Vec<ScannedPlugin>, ScanFailure> { panic!("no scan") };
        let missing = f.source.with_file_name("missing.clap");
        assert_eq!(
            install_file(&missing, &f.dest, false, &mut f.blocklist, never),
            Err(InstallError::NotFound)
        );
        let txt = f.source.with_file_name("readme.txt");
        std::fs::write(&txt, b"x").unwrap();
        assert_eq!(
            install_file(&txt, &f.dest, false, &mut f.blocklist, never),
            Err(InstallError::NotAPlugin)
        );
        std::fs::create_dir_all(&f.dest).unwrap();
        let installed = f.dest.join("acme-deesser.clap");
        std::fs::write(&installed, b"x").unwrap();
        assert_eq!(
            install_file(&installed, &f.dest, true, &mut f.blocklist, never),
            Err(InstallError::AlreadyInstalled)
        );
        assert_eq!(std::fs::read(&installed).unwrap(), b"x");
    }

    #[test]
    fn bundle_directories_copy_recursively() {
        let root = TempDir::new("bundle");
        let bundle = root.0.join("X.clap");
        std::fs::create_dir_all(bundle.join("Contents").join("MacOS")).unwrap();
        std::fs::write(bundle.join("Contents").join("MacOS").join("X"), b"bin").unwrap();
        std::fs::write(bundle.join("Contents").join("Info.plist"), b"plist").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink("MacOS/X", bundle.join("Contents").join("Current")).unwrap();
        let copy = root.0.join("copy.clap");
        copy_dir_all(&bundle, &copy).unwrap();
        assert_eq!(
            std::fs::read(copy.join("Contents").join("MacOS").join("X")).unwrap(),
            b"bin"
        );
        assert_eq!(
            std::fs::read(copy.join("Contents").join("Info.plist")).unwrap(),
            b"plist"
        );
        #[cfg(unix)]
        assert_eq!(
            std::fs::read_link(copy.join("Contents").join("Current")).unwrap(),
            PathBuf::from("MacOS/X")
        );
    }

    // --- T-806: VST3 bundles ----------------------------------------------------------------

    #[test]
    fn the_vst3_install_folder_is_per_user_and_picked_by_format() {
        let home = TempDir::new("vst3-home");
        let dir = user_vst3_dir_from(Some(home.0.as_os_str()), Some(home.0.as_os_str())).unwrap();
        assert!(dir.starts_with(&home.0), "{dir:?}");
        #[cfg(all(unix, not(target_os = "macos")))]
        assert_eq!(dir, home.0.join(".vst3"));
        assert_eq!(user_vst3_dir_from(None, None), None);
        assert_eq!(user_install_dir_for(Path::new("/x/readme.txt")), None);
    }

    #[test]
    fn installs_and_uninstalls_a_vst3_bundle() {
        let root = TempDir::new("vst3-install");
        let downloads = root.0.join("Downloads");
        let bundle = crate::scan::tests::vst3_bundle(
            &downloads,
            "Acme Gain",
            Some(crate::scan::tests::MODULEINFO),
        );
        let home = root.0.join("home");
        let dest = user_vst3_dir_from(Some(home.as_os_str()), Some(home.as_os_str())).unwrap();
        let mut blocklist = Blocklist::load(Some(root.0.join("blocklist.json")));
        let scanned = RefCell::new(Vec::new());
        let out = install_file(&bundle, &dest, false, &mut blocklist, |p| {
            scanned.borrow_mut().push(p.to_path_buf());
            Ok(vec![effect("84E8DE5F92554F5396FAE4133C935A18")])
        })
        .unwrap();
        let target = dest.join("Acme Gain.vst3");
        assert_eq!(out.target, target);
        assert_eq!(*scanned.borrow(), vec![target.clone()]);
        let binary = vox_sandbox_ipc::vst3::binary_path(&target).expect("the binary was copied");
        assert_eq!(std::fs::read(binary).unwrap(), b"binary");
        assert!(vox_sandbox_ipc::vst3::moduleinfo_path(&target).is_file());
        assert_eq!(
            visible_files(&dest),
            vec!["Acme Gain.vst3"],
            "no staging left behind"
        );

        // A file picked inside the bundle stands for the bundle: same name → a collision.
        let inside = vox_sandbox_ipc::vst3::moduleinfo_path(&bundle);
        assert_eq!(plugin_root(&inside), bundle);
        assert_eq!(
            install_file(&inside, &dest, false, &mut blocklist, |_| panic!("no scan")),
            Err(InstallError::Collision {
                target: target.clone()
            })
        );

        assert_eq!(uninstall_file(&target, &dest), Ok(()));
        assert!(!target.exists());
        assert!(bundle.exists(), "the picked bundle is untouched");
    }

    #[cfg(not(windows))]
    #[test]
    fn a_single_file_vst3_is_refused_outside_windows() {
        let mut f = fixture("vst3-single", b"x");
        let single = f.source.with_file_name("legacy.vst3");
        std::fs::write(&single, b"dll").unwrap();
        assert_eq!(
            install_file(&single, &f.dest, false, &mut f.blocklist, |_| panic!(
                "no scan"
            )),
            Err(InstallError::NotAPlugin)
        );
    }

    // --- T-807: LV2 bundles -----------------------------------------------------------------

    #[test]
    fn the_lv2_install_folder_is_per_user() {
        let home = TempDir::new("lv2-home");
        let dir = user_lv2_dir_from(Some(home.0.as_os_str()));
        #[cfg(all(unix, not(target_os = "macos")))]
        assert_eq!(dir, Some(home.0.join(".lv2")));
        #[cfg(target_os = "macos")]
        assert!(dir.unwrap().ends_with("Library/Audio/Plug-Ins/LV2"));
        #[cfg(windows)]
        assert_eq!(dir, None);
        assert_eq!(user_lv2_dir_from(None), None);
    }

    #[cfg(unix)]
    #[test]
    fn installs_and_uninstalls_an_lv2_bundle_picked_by_its_manifest() {
        let root = TempDir::new("lv2-install");
        let bundle = crate::scan::tests::lv2_bundle(&root.0.join("Downloads"), "Acme EQ", true);
        let home = root.0.join("home");
        let dest = user_lv2_dir_from(Some(home.as_os_str())).unwrap();
        let mut blocklist = Blocklist::load(Some(root.0.join("blocklist.json")));
        // A file dialog can't pick a folder: the manifest inside stands for the bundle.
        let manifest = bundle.join("manifest.ttl");
        assert_eq!(plugin_root(&manifest), bundle);
        let scanned = RefCell::new(Vec::new());
        let out = install_file(&manifest, &dest, false, &mut blocklist, |p| {
            scanned.borrow_mut().push(p.to_path_buf());
            Ok(vec![effect("urn:acme:eq")])
        })
        .unwrap();
        let target = dest.join("Acme EQ.lv2");
        assert_eq!(out.target, target);
        assert_eq!(*scanned.borrow(), vec![target.clone()]);
        assert!(target.join("manifest.ttl").is_file() && target.join("eq.so").is_file());
        assert_eq!(
            visible_files(&dest),
            vec!["Acme EQ.lv2"],
            "no staging left behind"
        );
        assert_eq!(uninstall_file(&target, &dest), Ok(()));
        assert!(!target.exists());
        assert!(bundle.exists(), "the picked bundle is untouched");
    }

    #[test]
    fn a_single_file_named_lv2_is_refused() {
        let mut f = fixture("lv2-single", b"x");
        let single = f.source.with_file_name("fake.lv2");
        std::fs::write(&single, b"not a bundle").unwrap();
        assert_eq!(
            install_file(&single, &f.dest, false, &mut f.blocklist, |_| panic!(
                "no scan"
            )),
            Err(InstallError::NotAPlugin)
        );
    }

    // --- H-29: "Uninstall…" -----------------------------------------------------------------

    #[test]
    fn uninstall_removes_a_file_inside_the_install_folder() {
        let f = fixture("uninstall", b"plugin");
        std::fs::create_dir_all(&f.dest).unwrap();
        let target = f.dest.join("acme-deesser.clap");
        std::fs::write(&target, b"plugin").unwrap();

        assert_eq!(uninstall_file(&target, &f.dest), Ok(()));
        assert!(!target.exists());
    }

    #[test]
    fn uninstall_removes_a_bundle_directory() {
        let root = TempDir::new("uninstall-bundle");
        let dest = root.0.join("dest");
        std::fs::create_dir_all(&dest).unwrap();
        let bundle = dest.join("X.clap");
        std::fs::create_dir_all(bundle.join("Contents").join("MacOS")).unwrap();
        std::fs::write(bundle.join("Contents").join("MacOS").join("X"), b"bin").unwrap();

        assert_eq!(uninstall_file(&bundle, &dest), Ok(()));
        assert!(!bundle.exists());
    }

    #[test]
    fn uninstall_refuses_a_missing_file() {
        let f = fixture("uninstall-missing", b"x");
        std::fs::create_dir_all(&f.dest).unwrap();
        let missing = f.dest.join("not-there.clap");
        assert_eq!(
            uninstall_file(&missing, &f.dest),
            Err(UninstallError::NotFound)
        );
    }

    /// The one thing that matters for H-29's trust boundary: a file living anywhere else — even
    /// right next to the install folder, sharing most of its path — is never removed.
    #[test]
    fn uninstall_refuses_a_file_outside_the_install_folder() {
        let f = fixture("uninstall-outside", b"plugin");
        // `f.source` (in `home/Downloads`) is never inside `f.dest` (`home/.clap`).
        assert_eq!(
            uninstall_file(&f.source, &f.dest),
            Err(UninstallError::OutsideInstallFolder)
        );
        assert!(f.source.exists(), "untouched");

        // A look-alike sibling folder (`.clap-extra`) is not a match by string prefix alone.
        let lookalike_dir = f.dest.with_file_name(format!(
            "{}-extra",
            f.dest.file_name().unwrap().to_string_lossy()
        ));
        std::fs::create_dir_all(&lookalike_dir).unwrap();
        let lookalike_file = lookalike_dir.join("acme-deesser.clap");
        std::fs::write(&lookalike_file, b"plugin").unwrap();
        assert_eq!(
            uninstall_file(&lookalike_file, &f.dest),
            Err(UninstallError::OutsideInstallFolder)
        );
        assert!(lookalike_file.exists(), "untouched");
    }

    /// A subfolder of the install folder isn't "inside" it for this purpose — every installed
    /// file is a direct child ([`install_file`] never nests one).
    #[test]
    fn uninstall_refuses_a_nested_subfolder_of_the_install_folder() {
        let f = fixture("uninstall-nested", b"plugin");
        std::fs::create_dir_all(&f.dest).unwrap();
        let nested_dir = f.dest.join("nested");
        std::fs::create_dir_all(&nested_dir).unwrap();
        let nested = nested_dir.join("acme.clap");
        std::fs::write(&nested, b"plugin").unwrap();
        assert_eq!(
            uninstall_file(&nested, &f.dest),
            Err(UninstallError::OutsideInstallFolder)
        );
        assert!(nested.exists());
    }

    #[test]
    fn uninstall_refuses_when_the_install_folder_does_not_exist() {
        let root = TempDir::new("uninstall-no-dir");
        let dest = root.0.join("never-created");
        let elsewhere = root.0.join("elsewhere.clap");
        std::fs::write(&elsewhere, b"plugin").unwrap();
        assert_eq!(
            uninstall_file(&elsewhere, &dest),
            Err(UninstallError::OutsideInstallFolder)
        );
        assert!(elsewhere.exists());
    }
}

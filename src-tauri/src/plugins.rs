//! Module registry composition (T-802, T-803, T-804, T-806): the built-in modules, the
//! **installed CLAP, VST3 and LV2 effects** — found in the standard CLAP/VST3/LV2 paths plus any configured
//! custom folders (VST3 bundles with `moduleinfo.json` indexed without loading code)
//! (ADR-008 §6), scanned in sandbox processes and cached by path + size + mtime, blocklisted on
//! a crash/timeout (ADR-008 §5) — and, behind the developer flag [`DEV_PLUGINS_ENV`]`=1`, the
//! sandboxed test plugins (`test:gain`, `test:crash`, `test:hang`). Every external plugin
//! instance runs in its own `powervoice-sandbox` process found next to the app executable
//! (`POWERVOICE_SANDBOX_BIN` overrides; `just dev` builds it).
//!
//! Start-up never blocks on a sandbox (T-804 item 1): [`configure`] loads only the scan cache
//! (a directory walk plus two small JSON files, no process spawned) before the first
//! [`registry`] call, and [`start_background_scan`] runs the authoritative scan afterwards on
//! its own thread, hot-adding anything new into every registry [`registry`] has ever returned —
//! including the ones `export`/`loudness`/`nr_capture` hold for the rest of the app's lifetime —
//! so Add Module (and every render) picks up a plugin found after start-up without a restart.
//!
//! **A `.voxmod` package's presets and locale strings** ([`module_presets`], [`module_locale`],
//! H-44, ADR-006 §3/§7 step 5): read fresh from `<modules>/<id>/<version>/` on every call, never
//! cached, so uninstalling a package (which deletes that folder) drops both for free.

use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use vox_plugin_host::health::HealthStore;
use vox_plugin_host::install::{InstallError, UninstallError};
use vox_plugin_host::scan::ShadowedPlugin;
use vox_plugin_host::{
    CatalogPaths, InstallReport, PluginCatalog, PluginDetails, SandboxOptions, SandboxSpec,
    ScanSummary,
};
use vox_rack::Registry;

/// `POWERVOICE_DEV_PLUGINS=1` adds the sandboxed test plugins to the Add-module menu.
pub const DEV_PLUGINS_ENV: &str = "POWERVOICE_DEV_PLUGINS";
/// `POWERVOICE_NO_PLUGIN_SCAN=1` skips the plugin scan (no external plugins in the registry).
pub const NO_PLUGIN_SCAN_ENV: &str = "POWERVOICE_NO_PLUGIN_SCAN";

fn flag_on(value: Option<OsString>) -> bool {
    value.is_some_and(|v| v == "1")
}

/// Whether the developer plugin flag is set.
pub fn dev_plugins_enabled() -> bool {
    flag_on(std::env::var_os(DEV_PLUGINS_ENV))
}

fn build(
    dev_plugins: bool,
    clap: &[SandboxSpec],
    options: &SandboxOptions,
) -> anyhow::Result<Registry> {
    let mut factories = vox_modules::builtin_factories();
    if dev_plugins {
        tracing::info!(sandbox = %options.binary.display(), "developer sandbox plugins enabled");
        factories.extend(vox_plugin_host::test_factories(options));
    }
    factories.extend(vox_plugin_host::scan::factories(clap, options));
    Ok(Registry::with_factories(factories)?)
}

fn project_dirs() -> Option<directories::ProjectDirs> {
    directories::ProjectDirs::from("app", "powervoice", "powervoice")
}

/// `<cache dir>/plugin-scan.json` (H-34: renamed from `clap-scan.json`, which held only CLAP
/// results before T-806 added VST3 to the same scan/cache). [`catalog`] migrates a leftover
/// legacy file the first time this path is used.
fn cache_path() -> Option<PathBuf> {
    project_dirs().map(|d| d.cache_dir().join(vox_plugin_host::scan::CACHE_FILE_NAME))
}

/// `<cache dir>/plugin-blocklist.json` (T-804, ADR-008 §5).
fn blocklist_path() -> Option<PathBuf> {
    project_dirs().map(|d| d.cache_dir().join("plugin-blocklist.json"))
}

/// `<cache dir>/plugin-health.json` (T-804, ADR-008 §5's runtime crash-flag counters).
fn health_path() -> Option<PathBuf> {
    project_dirs().map(|d| d.cache_dir().join("plugin-health.json"))
}

/// The process-wide plugin catalog (T-804): built once, lazily, on first use.
fn catalog() -> &'static Arc<PluginCatalog> {
    static CATALOG: OnceLock<Arc<PluginCatalog>> = OnceLock::new();
    CATALOG.get_or_init(|| {
        let mut sandbox_options = SandboxOptions::beside_current_exe();
        sandbox_options.health = Some(Arc::new(HealthStore::load(health_path())));
        let cache = cache_path();
        if let Some(cache) = &cache {
            vox_plugin_host::scan::migrate_cache_file_name(cache);
        }
        let catalog = PluginCatalog::new(
            sandbox_options,
            CatalogPaths {
                cache,
                blocklist: blocklist_path(),
            },
        );
        // T-805 (ADR-006 §4): built-ins are never loaded from CLAP, and no package can claim a
        // built-in's id.
        catalog.set_reserved_ids(
            vox_modules::builtin_factories()
                .iter()
                .map(|f| f.descriptor().id.clone())
                .collect(),
        );
        Arc::new(catalog)
    })
}

/// T-805 (ADR-006 §6): the per-user modules folder `.voxmod` packages install into —
/// `<app local data dir>/modules` (Tauri's `app_local_data_dir()`, passed in by the setup hook).
/// Call before [`configure`], so the cached load already sees installed modules.
pub fn set_modules_dir(dir: Option<PathBuf>) {
    catalog().set_modules_dir(dir);
}

/// The per-user modules folder, if the app has one (T-805).
pub fn modules_dir() -> Option<PathBuf> {
    catalog().modules_dir()
}

/// The runtime crash-flag store (T-804 item 4; for `plugins_list`).
pub fn health() -> Option<Arc<HealthStore>> {
    catalog().health().cloned()
}

/// The catalog's current effect specs (T-804 item 6; for `plugins_list`).
pub fn registry_specs() -> Vec<SandboxSpec> {
    catalog().specs()
}

/// The catalog's blocklist snapshot (T-804 item 6; for `plugins_list`).
pub fn blocklist_snapshot() -> Vec<(PathBuf, vox_plugin_host::blocklist::BlockInfo)> {
    catalog().blocklist_snapshot()
}

/// The catalog's duplicate-id losers (H-29 item 6; for `plugins_list`'s "Shadowed by …" rows).
pub fn shadowed_plugins() -> Vec<ShadowedPlugin> {
    catalog().shadowed()
}

/// Blocks `path` manually (`plugins_block`).
pub fn block(path: &std::path::Path) {
    catalog().block(path);
}

/// Unblocks `path` (`plugins_unblock`); `true` if it was blocked.
pub fn unblock(path: &std::path::Path) -> bool {
    catalog().unblock(path)
}

/// One-time start-up configuration (T-804 item 5): the custom folders from
/// `Settings.plugins.custom_folders`. Also does the **instant, sandbox-free** cached-plugins
/// load (T-804 item 1) — call this once, before the first [`registry`], and again whenever the
/// custom folders change (`plugins_add_folder`/`plugins_remove_folder`).
pub fn configure(custom_folders: &[String]) {
    catalog().set_custom_folders(custom_folders.iter().map(PathBuf::from).collect());
    catalog().load_cached();
}

/// The registry of a composition root: builtins, the developer test plugins (behind the flag),
/// and every CLAP effect the catalog currently knows (cache-only until the first
/// [`start_background_scan`] finishes). The returned registry is registered with the catalog, so
/// it keeps receiving hot-added plugins for as long as it lives (T-804 item 1) — every
/// composition root should call this once and hold onto the result, the same way `audio::start`,
/// `export::start`, `loudness::start` and `nr_capture::start` already do.
pub fn registry() -> anyhow::Result<Arc<Registry>> {
    let cat = catalog();
    let r = Arc::new(build(
        dev_plugins_enabled(),
        &cat.specs(),
        cat.sandbox_options(),
    )?);
    cat.observe(&r);
    Ok(r)
}

/// Starts the background rescan (T-804 item 1): finds every CLAP file (standard paths + custom
/// folders), scans the ones the cache doesn't already know, hot-adds newly found effects into
/// every live registry [`registry`] has returned, and reports progress/summary through `report`
/// (wired to the `plugin_scan_progress` event and its final summary by the caller).
///
/// A no-op (after `report`'s optional "disabled" call, left to the caller) when
/// [`NO_PLUGIN_SCAN_ENV`] is set.
pub fn start_background_scan(
    on_progress: impl Fn(usize, usize, &std::path::Path) + Send + Sync + 'static,
    on_done: impl FnOnce(ScanSummary) + Send + 'static,
) {
    if flag_on(std::env::var_os(NO_PLUGIN_SCAN_ENV)) {
        tracing::info!("plugin scan disabled");
        on_done(ScanSummary::default());
        return;
    }
    catalog().rescan_in_background(false, on_progress, on_done);
}

/// `plugins_rescan`'s synchronous, optionally-"full" rescan (T-804 item 6), reporting progress
/// (T-809: the plugin manager's progress bar).
pub fn rescan_with(
    full: bool,
    on_progress: impl Fn(usize, usize, &std::path::Path) + Sync,
) -> ScanSummary {
    catalog().rescan(full, on_progress)
}

/// What the last scan learned about `module_id` (ports, parameter count; T-809).
pub fn details(module_id: &str) -> Option<PluginDetails> {
    catalog().details(module_id)
}

/// Clears `module_id`'s runtime crash flag (T-809).
pub fn clear_flag(module_id: &str) {
    catalog().clear_flag(module_id);
}

/// `module_id`'s factory presets from its `.voxmod` package's `presets/*.vopreset.json` (H-44,
/// ADR-006 §3/§7 step 5) — empty unless it was installed from a package that shipped some. Read
/// fresh from disk every call, so an uninstall drops them without anything to invalidate.
pub fn module_presets(module_id: &str) -> Vec<vox_plugin_host::ModulePreset> {
    catalog().module_presets(module_id)
}

/// `module_id`'s validated `locales/<lang>.json` strings (H-44, ADR-006 §3/§7 step 5, Amendment
/// 3). `Ok(None)`: not installed from a package, or it has no file for `lang`. `Err`: the file
/// exists but failed validation — the caller shows a notice; nothing else fails because of it.
pub fn module_locale(
    module_id: &str,
    lang: &str,
) -> Result<
    Option<std::collections::BTreeMap<String, String>>,
    vox_plugin_host::module_locale::LocaleError,
> {
    catalog().module_locale(module_id, lang)
}

/// Every module id currently installed from a `.voxmod` package (H-44) — for merging every
/// installed package's locale strings at once.
pub fn installed_module_ids() -> Vec<String> {
    catalog().installed_module_ids()
}

/// The per-user CLAP folder (T-809; ADR-006 Amendment 1): `~/.clap`,
/// `~/Library/Audio/Plug-Ins/CLAP` or `%LOCALAPPDATA%\Programs\Common\CLAP`.
pub fn install_dir() -> Option<PathBuf> {
    vox_plugin_host::install::user_clap_dir()
}

/// Every per-user folder "Install module…" copies into, one per format (T-806: the CLAP folder
/// and `~/.vst3`, `~/Library/Audio/Plug-Ins/VST3` or `%LOCALAPPDATA%\Programs\Common\VST3`;
/// T-807: `~/.lv2` or `~/Library/Audio/Plug-Ins/LV2`, none on Windows; T-808: PowerVoice's JSFX
/// folder, `<data dir>/Effects`, none on Windows; T-805: the modules folder `.voxmod` packages
/// extract into).
pub fn install_dirs() -> Vec<PathBuf> {
    [
        vox_plugin_host::install::user_clap_dir(),
        vox_plugin_host::install::user_vst3_dir(),
        vox_plugin_host::install::user_lv2_dir(),
        vox_plugin_host::install::user_jsfx_dir(),
        modules_dir(),
    ]
    .into_iter()
    .flatten()
    .collect()
}

/// The standard per-format folders every scan searches (`$CLAP_PATH`, then the CLAP folders;
/// `$VST3_PATH`, then the VST3 folders; `$LV2_PATH`, then the LV2 folders — T-807; REAPER's
/// effects folder when it exists — T-808).
pub fn standard_folders() -> Vec<PathBuf> {
    let mut dirs = vox_plugin_host::scan::clap_search_paths();
    let others = vox_plugin_host::scan::vst3_search_paths()
        .into_iter()
        .chain(vox_plugin_host::scan::lv2_search_paths())
        .chain(vox_plugin_host::scan::jsfx_search_paths());
    for d in others {
        if !dirs.contains(&d) {
            dirs.push(d);
        }
    }
    dirs
}

/// "Install module…" (T-809, T-806): copies `source` into the per-user folder of its format —
/// never a system folder — and scans only that file (`vox_plugin_host::PluginCatalog::install`).
/// T-805: a `.voxmod` package is validated without running code and extracted into the modules
/// folder instead (`PluginCatalog::install_module`).
pub fn install(source: &std::path::Path, replace: bool) -> Result<InstallReport, InstallError> {
    if vox_plugin_host::install::is_voxmod(source) {
        return catalog().install_module(source, replace);
    }
    let root = vox_plugin_host::install::plugin_root(source);
    if vox_plugin_host::scan::format_of_path(&root).is_none() {
        return Err(InstallError::NotAPlugin);
    }
    let dir =
        vox_plugin_host::install::user_install_dir_for(&root).ok_or(InstallError::NoInstallDir)?;
    catalog().install(source, &dir, replace)
}

/// "Uninstall…" (H-29, T-806): removes `path` from the per-user folder of its format — never a
/// system folder — and drops it from the registry and the scan cache
/// (`vox_plugin_host::PluginCatalog::uninstall`).
pub fn uninstall(path: &std::path::Path) -> Result<(), UninstallError> {
    // T-805: a module installed from a package lives in `<modules>/<id>/<version>/`.
    if let Some(modules) = modules_dir()
        && vox_plugin_host::install::module_dir_of(path, &modules).is_ok()
    {
        return catalog().uninstall_module(path, &modules);
    }
    if vox_plugin_host::scan::format_of_path(path).is_none() {
        return Err(UninstallError::OutsideInstallFolder);
    }
    let dir =
        vox_plugin_host::install::user_install_dir_for(path).ok_or(UninstallError::NoInstallDir)?;
    catalog().uninstall(path, &dir)
}

/// The OS command that shows `path` in the file manager (T-809): macOS selects it in Finder,
/// Windows in Explorer; elsewhere the containing folder opens (`xdg-open`; selecting a file
/// needs a desktop-specific D-Bus call).
pub fn reveal_command(path: &std::path::Path) -> (&'static str, Vec<OsString>) {
    if cfg!(target_os = "macos") {
        ("open", vec!["-R".into(), path.as_os_str().to_owned()])
    } else if cfg!(windows) {
        let mut arg = OsString::from("/select,");
        arg.push(path.as_os_str());
        ("explorer", vec![arg])
    } else {
        let dir = if path.is_dir() {
            path
        } else {
            path.parent().unwrap_or(path)
        };
        ("xdg-open", vec![dir.as_os_str().to_owned()])
    }
}

/// Shows `path` in the file manager (T-809); the child is reaped on its own thread.
pub fn reveal(path: &std::path::Path) -> std::io::Result<()> {
    let (program, args) = reveal_command(path);
    let mut child = std::process::Command::new(program)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()?;
    std::thread::Builder::new()
        .name("reveal-wait".into())
        .spawn(move || {
            let _ = child.wait();
        })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use vox_plugin_host::scan::ScannedPlugin;
    use vox_plugin_host::{clap_spec, vst3_spec};

    #[test]
    fn the_dev_flag_adds_the_sandboxed_test_plugins() {
        assert!(flag_on(Some("1".into())));
        assert!(!flag_on(Some("0".into())));
        assert!(!flag_on(Some("yes".into())));
        assert!(!flag_on(None));
        let options = SandboxOptions::beside_current_exe();
        let plain = build(false, &[], &options).unwrap();
        assert!(plain.get("org.powervoice.gain").is_some());
        assert!(plain.get("test:gain").is_none());
        let dev = build(true, &[], &options).unwrap();
        for id in ["test:gain", "test:crash", "test:hang"] {
            assert!(dev.get(id).is_some(), "{id}");
        }
        assert_eq!(dev.len(), plain.len() + 3);
    }

    #[test]
    fn reveal_opens_the_containing_folder_or_selects_the_file() {
        let (program, args) = reveal_command(std::path::Path::new("/home/u/.clap/acme.clap"));
        if cfg!(target_os = "macos") {
            assert_eq!(program, "open");
            assert_eq!(args[0], "-R");
        } else if cfg!(windows) {
            assert_eq!(program, "explorer");
        } else {
            assert_eq!(program, "xdg-open");
            assert_eq!(args, vec![OsString::from("/home/u/.clap")]);
        }
    }

    #[test]
    fn scanned_clap_effects_are_registered_without_the_dev_flag() {
        let spec = clap_spec(
            std::path::Path::new("/usr/lib/clap/acme.clap"),
            &ScannedPlugin {
                id: "com.acme.deesser".into(),
                name: "De-esser".into(),
                vendor: "Acme".into(),
                version: "2.1".into(),
                description: String::new(),
                url: None,
                features: vec!["audio-effect".into()],
                ..ScannedPlugin::default()
            },
        );
        let r = build(false, &[spec], &SandboxOptions::beside_current_exe()).unwrap();
        let f = r.get("clap:com.acme.deesser").unwrap();
        assert!(f.loads_async());
        assert_eq!(f.descriptor().name.text, "De-esser");
    }

    #[test]
    fn scanned_vst3_effects_are_registered_too() {
        let spec = vst3_spec(
            std::path::Path::new("/usr/lib/vst3/Acme.vst3"),
            &ScannedPlugin {
                id: "84E8DE5F92554F5396FAE4133C935A18".into(),
                name: "Acme EQ".into(),
                vendor: "Acme".into(),
                version: "2.0".into(),
                features: vec!["audio-effect".into(), "equalizer".into()],
                ..ScannedPlugin::default()
            },
        );
        let r = build(false, &[spec], &SandboxOptions::beside_current_exe()).unwrap();
        let f = r.get("vst3:84E8DE5F92554F5396FAE4133C935A18").unwrap();
        assert!(f.loads_async());
        assert_eq!(f.descriptor().name.text, "Acme EQ");
    }

    #[test]
    fn install_and_standard_folders_cover_both_formats() {
        let standard = standard_folders();
        #[cfg(all(unix, not(target_os = "macos")))]
        {
            assert!(standard.contains(&PathBuf::from("/usr/lib/clap")));
            assert!(standard.contains(&PathBuf::from("/usr/lib/vst3")));
            assert!(standard.contains(&PathBuf::from("/usr/lib/lv2")));
        }
        if std::env::var_os("HOME").is_some() && cfg!(all(unix, not(target_os = "macos"))) {
            let dirs = install_dirs();
            assert!(dirs.iter().any(|d| d.ends_with(".clap")));
            assert!(dirs.iter().any(|d| d.ends_with(".vst3")));
            assert!(dirs.iter().any(|d| d.ends_with(".lv2")));
        }
        assert!(matches!(
            install(std::path::Path::new("/tmp/readme.txt"), false),
            Err(InstallError::NotAPlugin)
        ));
        assert_eq!(
            uninstall(std::path::Path::new("/tmp/readme.txt")),
            Err(UninstallError::OutsideInstallFolder)
        );
    }
}

//! Module registry composition (T-802, T-803, T-804): the built-in modules, the **installed
//! CLAP effects** — found in the standard CLAP paths plus any configured custom folders
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

use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use vox_plugin_host::health::HealthStore;
use vox_plugin_host::{CatalogPaths, PluginCatalog, SandboxOptions, SandboxSpec, ScanSummary};
use vox_rack::Registry;

/// `POWERVOICE_DEV_PLUGINS=1` adds the sandboxed test plugins to the Add-module menu.
pub const DEV_PLUGINS_ENV: &str = "POWERVOICE_DEV_PLUGINS";
/// `POWERVOICE_NO_PLUGIN_SCAN=1` skips the CLAP scan (no external plugins in the registry).
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

/// `<cache dir>/clap-scan.json`.
fn cache_path() -> Option<PathBuf> {
    project_dirs().map(|d| d.cache_dir().join("clap-scan.json"))
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
        Arc::new(PluginCatalog::new(
            sandbox_options,
            CatalogPaths {
                cache: cache_path(),
                blocklist: blocklist_path(),
            },
        ))
    })
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
        tracing::info!("CLAP plugin scan disabled");
        on_done(ScanSummary::default());
        return;
    }
    catalog().rescan_in_background(false, on_progress, on_done);
}

/// `plugins_rescan`'s synchronous, optionally-"full" rescan (T-804 item 6).
pub fn rescan(full: bool) -> ScanSummary {
    catalog().rescan(full, |_, _, _| {})
}

#[cfg(test)]
mod tests {
    use super::*;
    use vox_plugin_host::clap_spec;
    use vox_plugin_host::scan::ScannedPlugin;

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
}

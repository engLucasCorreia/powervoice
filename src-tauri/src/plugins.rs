//! Module registry composition (T-802, T-803): the built-in modules, the **installed CLAP
//! effects** (T-803: found in the standard CLAP paths, each file scanned in its own
//! `powervoice-sandbox --scan` process and cached by path + size + mtime), and — behind the
//! developer flag [`DEV_PLUGINS_ENV`]`=1` — the sandboxed test plugins (`test:gain`,
//! `test:crash`, `test:hang`). Every external plugin instance runs in its own
//! `powervoice-sandbox` process found next to the app executable (`POWERVOICE_SANDBOX_BIN`
//! overrides; `just dev` builds it). Every composition root (engine, export, loudness analysis,
//! noise-print capture) uses the same registry — the scan runs once per app start — so a rack
//! with plugin slots renders everywhere it plays.

use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::Instant;

use vox_plugin_host::scan::{self, ScanOptions};
use vox_plugin_host::{SandboxOptions, SandboxSpec};
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

fn build(dev_plugins: bool, clap: &[SandboxSpec]) -> anyhow::Result<Registry> {
    let mut factories = vox_modules::builtin_factories();
    let options = SandboxOptions::beside_current_exe();
    if dev_plugins {
        tracing::info!(sandbox = %options.binary.display(), "developer sandbox plugins enabled");
        factories.extend(vox_plugin_host::test_factories(&options));
    }
    factories.extend(scan::factories(clap, &options));
    Ok(Registry::with_factories(factories)?)
}

/// The registry of every composition root.
pub fn registry() -> anyhow::Result<Registry> {
    build(dev_plugins_enabled(), clap_specs())
}

/// The installed CLAP effects, scanned once per process.
fn clap_specs() -> &'static [SandboxSpec] {
    static SPECS: OnceLock<Vec<SandboxSpec>> = OnceLock::new();
    SPECS.get_or_init(scan_clap)
}

/// `<cache dir>/clap-scan.json` (e.g. `~/.cache/powervoice/clap-scan.json`).
fn cache_path() -> Option<PathBuf> {
    directories::ProjectDirs::from("app", "powervoice", "powervoice")
        .map(|d| d.cache_dir().join("clap-scan.json"))
}

fn scan_clap() -> Vec<SandboxSpec> {
    if flag_on(std::env::var_os(NO_PLUGIN_SCAN_ENV)) {
        tracing::info!("CLAP plugin scan disabled");
        return Vec::new();
    }
    let dirs = scan::clap_search_paths();
    let files = scan::find_clap_files(&dirs);
    if files.is_empty() {
        tracing::info!(?dirs, "no CLAP plugins installed");
        return Vec::new();
    }
    let sandbox = SandboxOptions::beside_current_exe();
    let mut options = ScanOptions::new(&sandbox.binary);
    options.cache = cache_path();
    let t0 = Instant::now();
    let outcome = scan::scan_clap_files(&files, &options);
    for f in &outcome.failures {
        tracing::warn!(path = %f.path.display(), reason = %f.message, "CLAP plugin skipped");
    }
    let specs = scan::effect_specs(&outcome);
    tracing::info!(
        files = files.len(),
        scanned = outcome.scanned,
        cached = outcome.cached,
        failed = outcome.failures.len(),
        effects = specs.len(),
        ms = t0.elapsed().as_millis() as u64,
        "CLAP plugins scanned"
    );
    specs
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
        let plain = build(false, &[]).unwrap();
        assert!(plain.get("org.powervoice.gain").is_some());
        assert!(plain.get("test:gain").is_none());
        let dev = build(true, &[]).unwrap();
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
            },
        );
        let r = build(false, &[spec]).unwrap();
        let f = r.get("clap:com.acme.deesser").unwrap();
        assert!(f.loads_async());
        assert_eq!(f.descriptor().name.text, "De-esser");
    }
}

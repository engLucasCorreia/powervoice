//! Module registry composition (T-802): the built-in modules, plus — behind the developer flag
//! [`DEV_PLUGINS_ENV`]`=1` until T-803+ ship real plugin formats — the sandboxed test plugins
//! (`test:gain`, `test:crash`, `test:hang`), each running in its own `powervoice-sandbox`
//! process found next to the app executable (`POWERVOICE_SANDBOX_BIN` overrides; `just dev`
//! builds it). Every composition root (engine, export, loudness analysis, noise-print capture)
//! uses the same registry, so a rack with sandboxed slots renders everywhere it plays.

use std::ffi::OsString;

use vox_rack::Registry;

/// `POWERVOICE_DEV_PLUGINS=1` adds the sandboxed test plugins to the Add-module menu.
pub const DEV_PLUGINS_ENV: &str = "POWERVOICE_DEV_PLUGINS";

fn flag_on(value: Option<OsString>) -> bool {
    value.is_some_and(|v| v == "1")
}

/// Whether the developer plugin flag is set.
pub fn dev_plugins_enabled() -> bool {
    flag_on(std::env::var_os(DEV_PLUGINS_ENV))
}

fn build(dev_plugins: bool) -> anyhow::Result<Registry> {
    let mut factories = vox_modules::builtin_factories();
    if dev_plugins {
        let options = vox_plugin_host::SandboxOptions::beside_current_exe();
        tracing::info!(sandbox = %options.binary.display(), "developer sandbox plugins enabled");
        factories.extend(vox_plugin_host::test_factories(&options));
    }
    Ok(Registry::with_factories(factories)?)
}

/// The registry of every composition root.
pub fn registry() -> anyhow::Result<Registry> {
    build(dev_plugins_enabled())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_dev_flag_adds_the_sandboxed_test_plugins() {
        assert!(flag_on(Some("1".into())));
        assert!(!flag_on(Some("0".into())));
        assert!(!flag_on(Some("yes".into())));
        assert!(!flag_on(None));
        let plain = build(false).unwrap();
        assert!(plain.get("org.powervoice.gain").is_some());
        assert!(plain.get("test:gain").is_none());
        let dev = build(true).unwrap();
        for id in ["test:gain", "test:crash", "test:hang"] {
            assert!(dev.get(id).is_some(), "{id}");
        }
        assert_eq!(dev.len(), plain.len() + 3);
    }
}

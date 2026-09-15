//! `powervoice-sandbox --scan <file> --format clap` (ADR-008 §6, T-803; richer data T-804
//! Amendment 4): load one `.clap` file in this disposable process and list its plugins'
//! descriptors. A crashing or hanging file takes only this process down; the editor reports it
//! and moves on.
//!
//! For every plugin whose descriptor features look like an audio effect, T-804 additionally
//! instantiates it (main thread, [`ClapInstance::load`], never activated) to read its audio
//! ports and parameter count — the descriptor alone doesn't carry those. A plugin that fails to
//! instantiate here just keeps its zeroed port/parameter fields; it isn't fatal to the scan (a
//! plugin that crashes doing so still takes down only this disposable process, same as before).

use std::path::Path;

use vox_clap_abi::opt_str;
use vox_module_api::features;
use vox_sandbox_ipc::protocol::{ScanReport, ScannedPlugin};

use super::ClapInstance;
use super::library::ClapLibrary;

/// Whether `features` describes an audio effect (not an instrument or note effect) — mirrors
/// `vox_plugin_host::scan::is_effect`, kept as its own tiny copy here since the sandbox binary
/// doesn't link the editor-side `plugin-host` crate (ADR-008 §8: format code lives only here).
fn looks_like_effect(features: &[String]) -> bool {
    let has = |f: &str| features.iter().any(|x| x == f);
    has(features::AUDIO_EFFECT) && !has("instrument") && !has("note-effect")
}

/// The descriptors of every plugin in `path`.
pub fn scan(path: &Path) -> Result<ScanReport, String> {
    let library = ClapLibrary::open(path)?;
    let factory = library
        .plugin_factory()
        .ok_or_else(|| format!("{} has no CLAP plugin factory", path.display()))?;
    let (Some(count), Some(get)) = (factory.get_plugin_count, factory.get_plugin_descriptor) else {
        return Err(format!(
            "{} has an incomplete plugin factory",
            path.display()
        ));
    };
    let mut plugins = Vec::new();
    // SAFETY: the factory of a loaded, initialised library; main thread.
    let n = unsafe { count(factory) };
    for i in 0..n {
        // SAFETY: `i < count`; the descriptor lives as long as the library.
        let d = unsafe { get(factory, i) };
        if d.is_null() {
            continue;
        }
        // SAFETY: a valid descriptor; its strings are NUL-terminated or null.
        let (id, name, vendor, version, description, url, features) = unsafe {
            let d = &*d;
            let mut features = Vec::new();
            if !d.features.is_null() {
                let mut k = 0;
                while let Some(f) = opt_str(*d.features.add(k)) {
                    features.push(f);
                    k += 1;
                }
            }
            (
                opt_str(d.id),
                opt_str(d.name),
                opt_str(d.vendor),
                opt_str(d.version),
                opt_str(d.description),
                opt_str(d.url),
                features,
            )
        };
        let Some(id) = id.filter(|s| !s.is_empty()) else {
            continue;
        };
        let is_effect = looks_like_effect(&features);
        let mut plugin = ScannedPlugin {
            name: name.filter(|s| !s.is_empty()).unwrap_or_else(|| id.clone()),
            id,
            vendor: vendor.unwrap_or_default(),
            version: version.unwrap_or_default(),
            description: description.unwrap_or_default(),
            url: url.filter(|s| !s.is_empty()),
            features,
            param_count: 0,
            main_input_channels: 0,
            main_output_channels: 0,
        };
        // Richer data (T-804, ADR-008 §6): only audio effects have the ports the rack needs, and
        // only instantiating (not just the descriptor) reveals the channel counts and parameter
        // count. Best-effort: a plugin that fails to load this way just keeps zeroed fields.
        if is_effect && let Ok(inst) = ClapInstance::load(path, &plugin.id) {
            plugin.param_count = inst.param_count();
            (plugin.main_input_channels, plugin.main_output_channels) = inst.main_ports();
        }
        plugins.push(plugin);
    }
    Ok(ScanReport {
        path: path.to_string_lossy().into_owned(),
        plugins,
    })
}

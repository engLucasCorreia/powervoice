//! `powervoice-sandbox --scan <bundle> --format vst3` (ADR-008 §6, T-806): load one `.vst3`
//! module in this disposable process and list its audio processor classes. A crashing or
//! hanging module takes only this process down.
//!
//! Every class of category `Audio Module Class` is reported (instruments too — the editor
//! filters by features). For an audio effect the scan also instantiates it (never activated)
//! for its parameter count and main bus channels (T-804's richer scan data); a class that fails
//! to instantiate keeps zeroed fields. Bundles with `moduleinfo.json` are normally indexed by
//! the editor without reaching this code at all (Amendment 6 §5).

use std::path::Path;
use std::sync::Arc;

use vox_sandbox_ipc::protocol::{ScanReport, ScannedPlugin};
use vox_sandbox_ipc::vst3::{AUDIO_MODULE_CLASS, features};

use super::Vst3Instance;
use super::module::Vst3Module;
use super::uid;
use crate::clap::scan::looks_like_effect;

/// The audio processor classes of `path`.
pub fn scan(path: &Path) -> Result<ScanReport, String> {
    let module = Arc::new(Vst3Module::open(path)?);
    let url = (!module.url.is_empty()).then(|| module.url.clone());
    let mut plugins = Vec::new();
    for c in module
        .classes()
        .into_iter()
        .filter(|c| c.category == AUDIO_MODULE_CLASS)
    {
        let id = uid::to_hex(&c.cid);
        let mut plugin = ScannedPlugin {
            name: if c.name.is_empty() {
                id.clone()
            } else {
                c.name
            },
            id,
            vendor: c.vendor,
            version: c.version,
            description: String::new(),
            url: url.clone(),
            features: features(&c.sub_categories),
            param_count: 0,
            main_input_channels: 0,
            main_output_channels: 0,
            module_info: None,
        };
        if looks_like_effect(&plugin.features)
            && let Ok(inst) = Vst3Instance::load_in(module.clone(), &plugin.id)
        {
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

//! `powervoice-sandbox --scan <file> --format clap` (ADR-008 §6, T-803): load one `.clap` file
//! in this disposable process and list its plugins' descriptors. A crashing or hanging file
//! takes only this process down; the editor reports it and moves on.

use std::path::Path;

use vox_clap_abi::opt_str;
use vox_sandbox_ipc::protocol::{ScanReport, ScannedPlugin};

use super::library::ClapLibrary;

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
        plugins.push(ScannedPlugin {
            name: name.filter(|s| !s.is_empty()).unwrap_or_else(|| id.clone()),
            id,
            vendor: vendor.unwrap_or_default(),
            version: version.unwrap_or_default(),
            description: description.unwrap_or_default(),
            url: url.filter(|s| !s.is_empty()),
            features,
        });
    }
    Ok(ScanReport {
        path: path.to_string_lossy().into_owned(),
        plugins,
    })
}

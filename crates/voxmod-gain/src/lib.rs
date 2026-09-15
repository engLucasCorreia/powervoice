//! The example **module package** (T-805, ADR-006 §4): PowerVoice's built-in Gain
//! (`vox_modules::Gain`, unchanged) exported as a standalone CLAP plugin through
//! [`vox_module_clap`]. It loads in PowerVoice (sandboxed, like any installed module) and in any
//! other CLAP host.
//!
//! - Id [`ID`] = `org.powervoice.gain.packaged`: the `.packaged` suffix keeps it apart from the
//!   built-in `org.powervoice.gain` — built-ins are never loaded from CLAP, and a package can
//!   never claim a built-in id (the install flow refuses it).
//! - Everything else (parameters, state format, processing) is the built-in's, so its output is
//!   bit-identical to the in-process Gain after the host's reported latency, and a state saved by
//!   one loads in the other.
//! - `voxmod.json` next to this crate's `Cargo.toml` is the package manifest template
//!   `just voxmod voxmod-gain` fills in (binaries, checksums).

use vox_module_api::{LocalizedText, Module, ModuleDescriptor, ModuleError, ModuleFactory};
use vox_modules::Gain;

/// The packaged module's id (also its CLAP plugin id).
pub const ID: &str = "org.powervoice.gain.packaged";
/// Its display name.
pub const NAME: &str = "Gain (packaged)";

/// Factory of the packaged Gain: the built-in's descriptor under [`ID`] and [`NAME`].
pub struct PackagedGainFactory {
    descriptor: ModuleDescriptor,
}

impl Default for PackagedGainFactory {
    fn default() -> Self {
        let mut descriptor = Gain::descriptor_value();
        descriptor.id = ID.into();
        descriptor.name = LocalizedText::plain(NAME);
        descriptor.description =
            LocalizedText::plain("PowerVoice's Gain, installed as a separate CLAP module package.");
        descriptor.url = Some("https://powervoice.app".into());
        Self { descriptor }
    }
}

impl ModuleFactory for PackagedGainFactory {
    fn descriptor(&self) -> &ModuleDescriptor {
        &self.descriptor
    }

    fn create(&self) -> Result<Box<dyn Module>, ModuleError> {
        Ok(Box::new(Gain::new()))
    }
}

vox_module_clap::export_module!(PackagedGainFactory);

#[cfg(test)]
mod tests {
    use super::*;

    /// The manifest template and the plugin must agree (the install flow checks the scanned id
    /// and version against the manifest).
    #[test]
    fn the_manifest_template_matches_the_factory() {
        let manifest: serde_json::Value =
            serde_json::from_str(include_str!("../voxmod.json")).unwrap();
        let d = PackagedGainFactory::default().descriptor;
        assert_eq!(manifest["id"], d.id.as_str());
        assert_eq!(manifest["version"], d.version.to_string().as_str());
        assert_eq!(manifest["name"], d.name.text.as_str());
        assert_eq!(manifest["vendor"], d.vendor.as_str());
        assert_eq!(manifest["module_api"], d.api_version);
    }

    #[test]
    fn the_id_is_a_module_id_and_not_the_built_ins() {
        assert!(vox_module_api::is_valid_module_id(ID));
        assert_ne!(ID, Gain::ID);
        let d = PackagedGainFactory::default().descriptor;
        assert_eq!(d.state_format_version, Gain::STATE_FORMAT_VERSION);
    }
}

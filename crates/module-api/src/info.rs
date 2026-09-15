//! What a packaged module tells its host beyond plain CLAP (ADR-006 §2, T-805).
//!
//! A PowerVoice module shipped as a CLAP plugin (`vox-module-clap`) answers the vendor extension
//! [`MODULE_INFO_EXTENSION_ID`] with **UTF-8 JSON of [`ModuleInfo`]**: the ADR-005 types
//! themselves, so the sidecar, the IPC and the package share one serde schema. Schema evolution
//! is additive JSON fields; a breaking change bumps the id to `/2`. Hosts parse the JSON with
//! `serde_json` (this crate stays free of it) and then call [`ModuleInfo::validate`].

use serde::{Deserialize, Serialize};

use crate::descriptor::{MODULE_API_VERSION, ModuleDescriptor, is_valid_module_id};
use crate::module::Module;
use crate::param::{ParamGroup, ParamInfo, validate_schema};

/// The CLAP vendor extension id of [`ModuleInfo`] (ADR-006 §2).
pub const MODULE_INFO_EXTENSION_ID: &str = "org.powervoice.module-info/1";

/// Everything CLAP can't express about a packaged module (ADR-006 §2): parameter keys, i18n
/// keys, tapers, units, decimals, `smoothing_ms`, enum labels, groups with `enable_param`,
/// `state_format_version`, `api_version`, and which other PowerVoice extensions are present.
///
/// `params[i].id` is the parameter's CLAP id: the standard `params` extension and this list
/// must agree on ids, ranges, defaults and flags (the host checks it before trusting either).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ModuleInfo {
    /// The module's descriptor. `id` equals the CLAP plugin id.
    pub descriptor: ModuleDescriptor,
    /// Parameter schema (display order).
    pub params: Vec<ParamInfo>,
    /// Parameter groups (display order).
    #[serde(default)]
    pub groups: Vec<ParamGroup>,
    /// Ids of the other PowerVoice CLAP extensions the plugin implements
    /// (`org.powervoice.telemetry/1`, …); empty when it has none.
    #[serde(default)]
    pub extensions: Vec<String>,
}

/// Why a [`ModuleInfo`] can't be trusted.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ModuleInfoError {
    /// The id isn't a reverse-DNS module id.
    #[error("`{0}` isn't a valid module id")]
    Id(String),
    /// Built against a newer Module API than this host implements (or a nonsensical one).
    #[error("module API version {0} isn't supported (this host implements {MODULE_API_VERSION})")]
    ApiVersion(u32),
    /// The parameter schema violates ADR-005 §3.
    #[error("invalid parameter schema: {0}")]
    Schema(String),
}

impl ModuleInfo {
    /// The info of `module` published under `descriptor` (a packaged module's descriptor may
    /// differ from the in-process one: its id carries a suffix such as `.packaged`).
    pub fn of(descriptor: ModuleDescriptor, module: &dyn Module) -> Self {
        Self {
            descriptor,
            params: module.params().to_vec(),
            groups: module.groups().to_vec(),
            extensions: Vec::new(),
        }
    }

    /// Checks the id format, the API version (1 ..= [`MODULE_API_VERSION`]) and the schema.
    pub fn validate(&self) -> Result<(), ModuleInfoError> {
        if !is_valid_module_id(&self.descriptor.id) {
            return Err(ModuleInfoError::Id(self.descriptor.id.clone()));
        }
        let api = self.descriptor.api_version;
        if api == 0 || api > MODULE_API_VERSION {
            return Err(ModuleInfoError::ApiVersion(api));
        }
        validate_schema(&self.params, &self.groups)
            .map_err(|e| ModuleInfoError::Schema(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::descriptor::{LocalizedText, Version};
    use crate::param::{ParamFlags, ParamId, Taper, Unit};

    fn info() -> ModuleInfo {
        ModuleInfo {
            descriptor: ModuleDescriptor {
                id: "com.acme.deesser".into(),
                version: Version::new(1, 2, 0),
                name: LocalizedText::plain("De-esser"),
                vendor: "Acme".into(),
                description: LocalizedText::plain(""),
                url: None,
                features: vec!["audio-effect".into(), "mono".into()],
                state_format_version: 3,
                api_version: MODULE_API_VERSION,
            },
            params: vec![ParamInfo {
                id: ParamId(7),
                key: "amount_db".into(),
                name: LocalizedText::keyed("modules.com.acme.deesser.amount", "Amount"),
                group: None,
                unit: Unit::Db,
                min: -24.0,
                max: 0.0,
                default: -6.0,
                taper: Taper::Db {
                    neg_inf_at_min: false,
                },
                step: None,
                enum_labels: Vec::new(),
                decimals: 1,
                smoothing_ms: 10.0,
                flags: ParamFlags::AUTOMATABLE,
            }],
            groups: Vec::new(),
            extensions: Vec::new(),
        }
    }

    #[test]
    fn json_round_trips_the_adr005_types() {
        let i = info();
        let json = serde_json::to_vec(&i).unwrap();
        let back: ModuleInfo = serde_json::from_slice(&json).unwrap();
        back.validate().unwrap();
        assert_eq!(back, i);
        // Additive evolution: unknown fields are ignored, missing optional ones default.
        let mut v: serde_json::Value = serde_json::from_slice(&json).unwrap();
        v["future_field"] = serde_json::json!(true);
        v.as_object_mut().unwrap().remove("groups");
        v.as_object_mut().unwrap().remove("extensions");
        assert_eq!(serde_json::from_value::<ModuleInfo>(v).unwrap(), i);
    }

    #[test]
    fn rejects_bad_ids_api_versions_and_schemas() {
        info().validate().unwrap();
        let mut i = info();
        i.descriptor.id = "clap:com.acme.deesser".into();
        assert!(matches!(i.validate(), Err(ModuleInfoError::Id(_))));
        let mut i = info();
        i.descriptor.api_version = MODULE_API_VERSION + 1;
        assert_eq!(
            i.validate(),
            Err(ModuleInfoError::ApiVersion(MODULE_API_VERSION + 1))
        );
        let mut i = info();
        i.params[0].default = 5.0;
        assert!(matches!(i.validate(), Err(ModuleInfoError::Schema(_))));
    }
}

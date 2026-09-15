//! PowerVoice modules packaged as CLAP plugins (ADR-006 §2, T-805): a plugin that answers the
//! `org.powervoice.module-info/1` vendor extension publishes its ADR-005 schema as JSON. Before
//! the host uses it instead of the generic CLAP mapping ([`super::params::map`]), the JSON must
//! be valid and **agree with the standard `params` extension** on ids, ranges, defaults and
//! flags — a plugin whose two descriptions disagree is refused (it would process one parameter
//! and show another).

use vox_clap_abi::*;
use vox_module_api::{ModuleInfo, ParamFlags};

use super::params::RawParam;

/// Parses and checks the module info `json` of plugin `id` against its CLAP parameters `raw`.
pub(crate) fn check(json: &[u8], id: &str, raw: &[RawParam]) -> Result<ModuleInfo, String> {
    let info: ModuleInfo = serde_json::from_slice(json).map_err(|e| e.to_string())?;
    info.validate().map_err(|e| e.to_string())?;
    if info.descriptor.id != id {
        return Err(format!(
            "it describes module `{}`, not `{id}`",
            info.descriptor.id
        ));
    }
    if raw.len() != info.params.len() {
        return Err(format!(
            "{} CLAP parameters but {} in the module info",
            raw.len(),
            info.params.len()
        ));
    }
    for p in &info.params {
        let Some(r) = raw.iter().find(|r| r.id == p.id.0) else {
            return Err(format!(
                "parameter `{}` (id {}) isn't a CLAP parameter",
                p.key, p.id.0
            ));
        };
        let same = |a: f64, b: f64| a.to_bits() == b.to_bits();
        if !(same(r.min, p.min) && same(r.max, p.max) && same(r.default, p.default)) {
            return Err(format!(
                "parameter `{}`: CLAP range {}..{} (default {}) vs {}..{} (default {})",
                p.key, r.min, r.max, r.default, p.min, p.max, p.default
            ));
        }
        let has = |f| p.flags.contains(f);
        let clap = |bit: u32| r.flags & bit != 0;
        let pairs = [
            (
                has(ParamFlags::AUTOMATABLE),
                clap(CLAP_PARAM_IS_AUTOMATABLE),
                "automatable",
            ),
            (
                has(ParamFlags::STEPPED) || has(ParamFlags::BOOL) || p.is_enum(),
                clap(CLAP_PARAM_IS_STEPPED),
                "stepped",
            ),
            (p.is_enum(), clap(CLAP_PARAM_IS_ENUM), "enum"),
            (
                has(ParamFlags::READ_ONLY),
                clap(CLAP_PARAM_IS_READONLY),
                "read-only",
            ),
            (
                has(ParamFlags::HIDDEN),
                clap(CLAP_PARAM_IS_HIDDEN),
                "hidden",
            ),
            (
                has(ParamFlags::BYPASS),
                clap(CLAP_PARAM_IS_BYPASS),
                "bypass",
            ),
        ];
        if let Some((_, _, what)) = pairs.iter().find(|(a, b, _)| a != b) {
            return Err(format!(
                "parameter `{}`: the CLAP and module-info `{what}` flags disagree",
                p.key
            ));
        }
    }
    Ok(info)
}

#[cfg(test)]
mod tests {
    use super::*;
    use vox_module_api::{
        LocalizedText, MODULE_API_VERSION, ModuleDescriptor, ParamId, ParamInfo, Taper, Unit,
        Version,
    };

    fn param() -> ParamInfo {
        ParamInfo {
            id: ParamId(3),
            key: "amount_db".into(),
            name: LocalizedText::plain("Amount"),
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
            smoothing_ms: 5.0,
            flags: ParamFlags::AUTOMATABLE,
        }
    }

    fn info() -> ModuleInfo {
        ModuleInfo {
            descriptor: ModuleDescriptor {
                id: "com.acme.x".into(),
                version: Version::new(1, 0, 0),
                name: LocalizedText::plain("X"),
                vendor: "Acme".into(),
                description: LocalizedText::plain(""),
                url: None,
                features: vec!["audio-effect".into(), "mono".into()],
                state_format_version: 1,
                api_version: MODULE_API_VERSION,
            },
            params: vec![param()],
            groups: Vec::new(),
            extensions: Vec::new(),
        }
    }

    fn raw() -> RawParam {
        RawParam {
            id: 3,
            flags: CLAP_PARAM_IS_AUTOMATABLE,
            name: "Amount".into(),
            module: String::new(),
            min: -24.0,
            max: 0.0,
            default: -6.0,
        }
    }

    fn json(i: &ModuleInfo) -> Vec<u8> {
        serde_json::to_vec(i).unwrap()
    }

    #[test]
    fn accepts_agreeing_descriptions() {
        let got = check(&json(&info()), "com.acme.x", &[raw()]).unwrap();
        assert_eq!(got, info());
    }

    #[test]
    fn refuses_bad_json_other_ids_and_disagreements() {
        assert!(check(b"{nope", "com.acme.x", &[raw()]).is_err());
        assert!(check(&json(&info()), "com.acme.y", &[raw()]).is_err());
        assert!(check(&json(&info()), "com.acme.x", &[]).is_err());
        let mut r = raw();
        r.id = 4;
        assert!(check(&json(&info()), "com.acme.x", &[r]).is_err());
        let mut r = raw();
        r.max = 6.0;
        assert!(check(&json(&info()), "com.acme.x", &[r]).is_err());
        let mut r = raw();
        r.flags |= CLAP_PARAM_IS_HIDDEN;
        let err = check(&json(&info()), "com.acme.x", &[r]).unwrap_err();
        assert!(err.contains("hidden"), "{err}");
        let mut bad = info();
        bad.params[0].key = "Bad Key".into();
        assert!(check(&json(&bad), "com.acme.x", &[raw()]).is_err());
    }
}

//! CLAP parameter info → the module API's schema (`ParamInfo`, `ParamGroup`), for the generic
//! parameter UI (ADR-008 Amendment 3 §4).
//!
//! - `id` = the CLAP id; `key` = `p<id>` (stable, valid per ADR-005 §3); `name` verbatim.
//! - Plain `min`/`max`/`default`, linear taper, unit `None`, no smoothing (the plugin smooths).
//!   Display text comes from the plugin (`ParamText`); `decimals` is only the fallback.
//! - Stepped → step 1; stepped 0..1 → `BOOL`; an enum 0..n (n < [`MAX_ENUM_LABELS`]) gets its
//!   labels from the plugin's `value_to_text`.
//! - Automatable → `AUTOMATABLE`; read-only → `READ_ONLY` (never automatable); hidden and the
//!   plugin's own bypass (the rack has host bypass) → `HIDDEN`.
//! - The first segment of the CLAP module path ("Filter/Cutoff" → "Filter") becomes a group.
//! - Degenerate ranges (non-finite, `min >= max`) become hidden read-only 0..1 parameters;
//!   duplicate ids are dropped.

use std::collections::HashSet;

use vox_module_api::{
    GroupId, LocalizedText, ParamFlags, ParamGroup, ParamId, ParamInfo, Taper, Unit,
};

/// Enums with more values than this stay plain stepped parameters.
pub(crate) const MAX_ENUM_LABELS: usize = 64;
/// More parameters than this: groups start collapsed.
const COLLAPSE_GROUPS_ABOVE: usize = 24;

/// One `clap_param_info`, copied out of the plugin.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RawParam {
    pub(crate) id: u32,
    pub(crate) flags: u32,
    pub(crate) name: String,
    pub(crate) module: String,
    pub(crate) min: f64,
    pub(crate) max: f64,
    pub(crate) default: f64,
}

fn decimals_for(range: f64) -> u8 {
    if range >= 100.0 {
        1
    } else if range >= 1.0 {
        2
    } else {
        3
    }
}

/// Maps `raw` to a schema; `label(id, value)` is the plugin's text for enum labels.
pub(crate) fn map(
    raw: &[RawParam],
    mut label: impl FnMut(u32, f64) -> Option<String>,
) -> (Vec<ParamInfo>, Vec<ParamGroup>) {
    use vox_clap_abi::*;
    let mut seen = HashSet::new();
    let mut params = Vec::with_capacity(raw.len());
    let mut groups: Vec<ParamGroup> = Vec::new();
    for r in raw {
        if !seen.insert(r.id) {
            continue;
        }
        let mut flags = ParamFlags::NONE;
        let (mut min, mut max) = (r.min, r.max);
        let degenerate = !(min.is_finite() && max.is_finite()) || min >= max;
        if degenerate {
            min = if min.is_finite() { min } else { 0.0 };
            max = min + 1.0;
        }
        let default = if r.default.is_finite() {
            r.default.clamp(min, max)
        } else {
            min
        };
        let read_only = degenerate || r.flags & CLAP_PARAM_IS_READONLY != 0;
        if read_only {
            flags |= ParamFlags::READ_ONLY;
        } else if r.flags & CLAP_PARAM_IS_AUTOMATABLE != 0 {
            flags |= ParamFlags::AUTOMATABLE;
        }
        if degenerate || r.flags & (CLAP_PARAM_IS_HIDDEN | CLAP_PARAM_IS_BYPASS) != 0 {
            flags |= ParamFlags::HIDDEN;
        }
        let stepped = r.flags & (CLAP_PARAM_IS_STEPPED | CLAP_PARAM_IS_ENUM) != 0;
        let mut step = None;
        let mut enum_labels = Vec::new();
        if stepped {
            flags |= ParamFlags::STEPPED;
            step = Some(1.0);
            let count = (max - min).round() as usize + 1;
            if min.to_bits() == 0f64.to_bits() && max.fract() == 0.0 {
                if count == 2 {
                    flags |= ParamFlags::BOOL;
                } else if r.flags & CLAP_PARAM_IS_ENUM != 0 && count <= MAX_ENUM_LABELS {
                    let labels: Option<Vec<LocalizedText>> = (0..count)
                        .map(|k| label(r.id, k as f64).map(LocalizedText::plain))
                        .collect();
                    enum_labels = labels.unwrap_or_default();
                }
            }
        }
        let group = r
            .module
            .split('/')
            .map(str::trim)
            .find(|s| !s.is_empty())
            .map(
                |segment| match groups.iter().find(|g| g.name.text == segment) {
                    Some(g) => g.id,
                    None => {
                        let id = GroupId(groups.len() as u32 + 1);
                        groups.push(ParamGroup {
                            id,
                            key: format!("g{}", id.0),
                            name: LocalizedText::plain(segment),
                            parent: None,
                            enable_param: None,
                            collapsed_by_default: false,
                        });
                        id
                    }
                },
            );
        let name = if r.name.trim().is_empty() {
            format!("Parameter {}", r.id)
        } else {
            r.name.clone()
        };
        params.push(ParamInfo {
            id: ParamId(r.id),
            key: format!("p{}", r.id),
            name: LocalizedText::plain(name),
            group,
            unit: Unit::None,
            min,
            max,
            default,
            taper: Taper::Linear,
            step,
            enum_labels,
            decimals: if stepped { 0 } else { decimals_for(max - min) },
            smoothing_ms: 0.0,
            flags,
        });
    }
    if params.len() > COLLAPSE_GROUPS_ABOVE {
        for g in &mut groups {
            g.collapsed_by_default = true;
        }
    }
    (params, groups)
}

#[cfg(test)]
mod tests {
    use super::*;
    use vox_clap_abi::*;
    use vox_module_api::validate_schema;

    fn raw(id: u32, flags: u32, module: &str, min: f64, max: f64, default: f64) -> RawParam {
        RawParam {
            id,
            flags,
            name: format!("P{id}"),
            module: module.into(),
            min,
            max,
            default,
        }
    }

    #[test]
    fn maps_flags_ranges_groups_and_enums() {
        let input = [
            raw(
                7,
                CLAP_PARAM_IS_AUTOMATABLE,
                "Filter/Cutoff",
                20.0,
                20_000.0,
                1000.0,
            ),
            raw(
                3,
                CLAP_PARAM_IS_STEPPED | CLAP_PARAM_IS_AUTOMATABLE,
                "",
                0.0,
                1.0,
                0.0,
            ),
            raw(
                9,
                CLAP_PARAM_IS_ENUM | CLAP_PARAM_IS_STEPPED,
                "Filter",
                0.0,
                2.0,
                1.0,
            ),
            raw(
                4,
                CLAP_PARAM_IS_READONLY | CLAP_PARAM_IS_AUTOMATABLE,
                "Meters",
                -1.0,
                1.0,
                0.0,
            ),
            raw(
                5,
                CLAP_PARAM_IS_BYPASS | CLAP_PARAM_IS_STEPPED,
                "",
                0.0,
                1.0,
                0.0,
            ),
            raw(6, 0, "", 1.0, 1.0, 1.0),
            raw(7, 0, "", 0.0, 1.0, 0.0),
            raw(8, 0, "", 0.0, f64::NAN, f64::INFINITY),
        ];
        let (params, groups) = map(&input, |id, v| (id == 9).then(|| format!("mode {v}")));
        validate_schema(&params, &groups).unwrap();
        assert_eq!(params.len(), 7, "duplicate id 7 dropped");
        let p = |id: u32| params.iter().find(|p| p.id == ParamId(id)).unwrap();
        assert_eq!(p(7).key, "p7");
        assert!(p(7).flags.contains(ParamFlags::AUTOMATABLE));
        assert_eq!((p(7).min, p(7).max, p(7).default), (20.0, 20_000.0, 1000.0));
        assert!(p(3).flags.contains(ParamFlags::BOOL));
        assert_eq!(p(9).enum_labels.len(), 3);
        assert_eq!(p(9).enum_labels[2].text, "mode 2");
        assert_eq!(p(9).group, p(7).group);
        assert!(p(4).flags.contains(ParamFlags::READ_ONLY));
        assert!(!p(4).flags.contains(ParamFlags::AUTOMATABLE));
        assert!(p(5).flags.contains(ParamFlags::HIDDEN));
        assert!(
            p(6).flags
                .contains(ParamFlags::HIDDEN | ParamFlags::READ_ONLY)
        );
        assert_eq!((p(6).min, p(6).max), (1.0, 2.0));
        assert_eq!((p(8).min, p(8).max, p(8).default), (0.0, 1.0, 0.0));
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].name.text, "Filter");
        assert_eq!(groups[1].name.text, "Meters");
    }
}

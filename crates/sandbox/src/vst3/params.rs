//! VST3 parameter info → the module API's schema (ADR-008 Amendment 6 §3).
//!
//! VST3 parameters are **normalized** (0..1) on the wire; the plugin's own plain domain is only
//! reachable through its controller (`normalizedParamToPlain`, a main-thread call), while the
//! host's parameter events reach the sandbox on its audio thread. So the mirror uses domains the
//! adapter can convert without the plugin, exactly, on any thread:
//! - **continuous** (`stepCount` 0) → min 0, max 1: the value *is* the normalized value;
//! - **discrete** (`stepCount` n > 0) → min 0, max n, step 1: the value is the step index, with
//!   the SDK's own conversions (`index / n`, `min(n, ⌊x·(n + 1)⌋)`); `n` = 1 → `BOOL`; a list
//!   (`kIsList`) of at most [`MAX_ENUM_LABELS`] entries gets its labels from the plugin.
//!
//! The plugin's text (`getParamStringByValue` + units) is what the UI shows either way.
//! - `key` = `p<id>`; `name` = the title; `kCanAutomate` → `AUTOMATABLE`; `kIsReadOnly` →
//!   `READ_ONLY`; `kIsHidden`, `kIsBypass` (the rack has host bypass) and `kIsProgramChange` →
//!   `HIDDEN`.
//! - A parameter in a unit other than the root (`IUnitInfo`) is grouped under the unit's name;
//!   groups start collapsed above 24 parameters. Duplicate ids are dropped.

use std::collections::HashSet;

use vox_module_api::{
    GroupId, LocalizedText, ParamFlags, ParamGroup, ParamId, ParamInfo, Taper, Unit,
};
use vst3::Steinberg::Vst::ParameterInfo_::ParameterFlags_::{
    kCanAutomate, kIsBypass, kIsHidden, kIsList, kIsProgramChange, kIsReadOnly,
};

/// Lists with more entries than this stay plain stepped parameters.
pub(crate) const MAX_ENUM_LABELS: u32 = 64;
/// More parameters than this: groups start collapsed.
const COLLAPSE_GROUPS_ABOVE: usize = 24;

/// How a mirror value maps to the plugin's normalized value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Domain {
    /// The value is the normalized value.
    Continuous,
    /// The value is a step index `0..=n`.
    Discrete(u32),
}

impl Domain {
    pub(crate) fn of(step_count: i32) -> Self {
        match u32::try_from(step_count) {
            Ok(n) if n > 0 => Self::Discrete(n),
            _ => Self::Continuous,
        }
    }

    /// Mirror value → normalized (RT-safe).
    pub(crate) fn to_normalized(self, v: f64) -> f64 {
        let v = if v.is_nan() { 0.0 } else { v };
        match self {
            Self::Continuous => v.clamp(0.0, 1.0),
            Self::Discrete(n) => v.round().clamp(0.0, f64::from(n)) / f64::from(n),
        }
    }

    /// Normalized → mirror value (RT-safe).
    pub(crate) fn value_of(self, x: f64) -> f64 {
        let x = if x.is_nan() { 0.0 } else { x.clamp(0.0, 1.0) };
        match self {
            Self::Continuous => x,
            Self::Discrete(n) => (x * f64::from(n + 1)).floor().min(f64::from(n)),
        }
    }
}

/// One `ParameterInfo`, copied out of the controller.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RawParam {
    pub(crate) id: u32,
    pub(crate) title: String,
    pub(crate) step_count: i32,
    pub(crate) default_normalized: f64,
    pub(crate) unit_id: i32,
    pub(crate) flags: i32,
}

fn has(flags: i32, flag: impl TryInto<i32>) -> bool {
    flag.try_into().is_ok_and(|f: i32| flags & f != 0)
}

/// Maps `raw` to a schema and per-parameter domains (`(id, domain)`, sorted by id). `units` =
/// `(unit id, name)` from `IUnitInfo`; `label(id, normalized)` is the plugin's text for list
/// labels.
pub(crate) fn map(
    raw: &[RawParam],
    units: &[(i32, String)],
    mut label: impl FnMut(u32, f64) -> Option<String>,
) -> (Vec<ParamInfo>, Vec<ParamGroup>, Vec<(u32, Domain)>) {
    let mut seen = HashSet::new();
    let mut params = Vec::with_capacity(raw.len());
    let mut groups: Vec<(i32, ParamGroup)> = Vec::new();
    let mut domains = Vec::with_capacity(raw.len());
    for r in raw {
        if !seen.insert(r.id) {
            continue;
        }
        let domain = Domain::of(r.step_count);
        let mut flags = ParamFlags::NONE;
        if has(r.flags, kIsReadOnly) {
            flags |= ParamFlags::READ_ONLY;
        } else if has(r.flags, kCanAutomate) {
            flags |= ParamFlags::AUTOMATABLE;
        }
        if has(r.flags, kIsHidden) || has(r.flags, kIsBypass) || has(r.flags, kIsProgramChange) {
            flags |= ParamFlags::HIDDEN;
        }
        let default = domain.value_of(r.default_normalized);
        let (max, step, decimals, enum_labels) = match domain {
            Domain::Continuous => (1.0, None, 3, Vec::new()),
            Domain::Discrete(n) => {
                flags |= ParamFlags::STEPPED;
                if n == 1 {
                    flags |= ParamFlags::BOOL;
                }
                let labels = if has(r.flags, kIsList) && n < MAX_ENUM_LABELS {
                    (0..=n)
                        .map(|k| {
                            label(r.id, domain.to_normalized(f64::from(k)))
                                .map(LocalizedText::plain)
                        })
                        .collect::<Option<Vec<_>>>()
                        .unwrap_or_default()
                } else {
                    Vec::new()
                };
                (f64::from(n), Some(1.0), 0, labels)
            }
        };
        let group = (r.unit_id != 0)
            .then(|| units.iter().find(|(id, _)| *id == r.unit_id))
            .flatten()
            .filter(|(_, name)| !name.trim().is_empty())
            .map(
                |(unit, name)| match groups.iter().find(|(u, _)| u == unit) {
                    Some((_, g)) => g.id,
                    None => {
                        let id = GroupId(groups.len() as u32 + 1);
                        groups.push((
                            *unit,
                            ParamGroup {
                                id,
                                key: format!("g{}", id.0),
                                name: LocalizedText::plain(name.trim()),
                                parent: None,
                                enable_param: None,
                                collapsed_by_default: false,
                            },
                        ));
                        id
                    }
                },
            );
        let name = if r.title.trim().is_empty() {
            format!("Parameter {}", r.id)
        } else {
            r.title.clone()
        };
        params.push(ParamInfo {
            id: ParamId(r.id),
            key: format!("p{}", r.id),
            name: LocalizedText::plain(name),
            group,
            unit: Unit::None,
            min: 0.0,
            max,
            default,
            taper: Taper::Linear,
            step,
            enum_labels,
            decimals,
            smoothing_ms: 0.0,
            flags,
        });
        domains.push((r.id, domain));
    }
    let mut groups: Vec<ParamGroup> = groups.into_iter().map(|(_, g)| g).collect();
    if params.len() > COLLAPSE_GROUPS_ABOVE {
        for g in &mut groups {
            g.collapsed_by_default = true;
        }
    }
    domains.sort_unstable_by_key(|d| d.0);
    (params, groups, domains)
}

#[cfg(test)]
#[allow(clippy::float_cmp)] // exact conversions are the point
mod tests {
    use super::*;
    use vox_module_api::validate_schema;

    fn raw(id: u32, step_count: i32, default: f64, unit_id: i32, flags: u32) -> RawParam {
        RawParam {
            id,
            title: format!("P{id}"),
            step_count,
            default_normalized: default,
            unit_id,
            flags: flags as i32,
        }
    }

    #[test]
    fn discrete_domains_use_the_sdk_conversions_exactly() {
        let d = Domain::Discrete(4096);
        for k in [0.0, 1.0, 64.0, 300.0, 4095.0, 4096.0] {
            assert_eq!(d.value_of(d.to_normalized(k)), k);
        }
        let b = Domain::Discrete(1);
        assert_eq!((b.value_of(0.49), b.value_of(0.5)), (0.0, 1.0));
        assert_eq!(Domain::of(0), Domain::Continuous);
        assert_eq!(Domain::of(-3), Domain::Continuous);
        let c = Domain::Continuous;
        assert_eq!(c.to_normalized(0.25).to_bits(), 0.25f64.to_bits());
        assert_eq!((c.to_normalized(2.0), c.value_of(f64::NAN)), (1.0, 0.0));
    }

    #[test]
    fn maps_flags_domains_units_and_lists() {
        let input = [
            raw(7, 0, 0.5, 0, kCanAutomate as u32),
            raw(3, 1, 1.0, 5, kCanAutomate as u32),
            raw(9, 2, 0.5, 5, kIsList as u32),
            raw(4, 0, 0.0, 6, (kIsReadOnly | kCanAutomate) as u32),
            raw(5, 1, 0.0, 0, kIsBypass as u32),
            raw(7, 0, 0.0, 0, 0),
        ];
        let units = [(5, "Filter".to_owned()), (6, "Meters".to_owned())];
        let (params, groups, domains) = map(&input, &units, |id, n| {
            (id == 9).then(|| format!("mode {n}"))
        });
        validate_schema(&params, &groups).unwrap();
        assert_eq!(params.len(), 5, "duplicate id 7 dropped");
        let p = |id: u32| params.iter().find(|p| p.id == ParamId(id)).unwrap();
        assert_eq!(
            (p(7).key.as_str(), p(7).min, p(7).max, p(7).default),
            ("p7", 0.0, 1.0, 0.5)
        );
        assert!(p(7).flags.contains(ParamFlags::AUTOMATABLE));
        assert_eq!(p(7).group, None);
        assert!(p(3).flags.contains(ParamFlags::BOOL | ParamFlags::STEPPED));
        assert_eq!(p(3).default, 1.0);
        assert_eq!((p(9).max, p(9).step, p(9).default), (2.0, Some(1.0), 1.0));
        assert_eq!(p(9).enum_labels.len(), 3);
        assert_eq!(p(9).enum_labels[2].text, "mode 1");
        assert_eq!(p(9).group, p(3).group);
        assert!(p(4).flags.contains(ParamFlags::READ_ONLY));
        assert!(!p(4).flags.contains(ParamFlags::AUTOMATABLE));
        assert!(p(5).flags.contains(ParamFlags::HIDDEN));
        assert_eq!(groups.len(), 2);
        assert_eq!(
            (groups[0].name.text.as_str(), groups[1].name.text.as_str()),
            ("Filter", "Meters")
        );
        assert_eq!(
            domains,
            vec![
                (3, Domain::Discrete(1)),
                (4, Domain::Continuous),
                (5, Domain::Discrete(1)),
                (7, Domain::Continuous),
                (9, Domain::Discrete(2)),
            ]
        );
    }
}

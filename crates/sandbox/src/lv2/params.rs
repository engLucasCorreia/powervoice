//! LV2 ports → the module API's schema (`ParamInfo`, `ParamGroup`), ADR-008 Amendment 8 §3.
//!
//! - Every control **input** port is a parameter; `ParamId` = the port index, `key` = the port
//!   symbol lower-cased (ADR-005 §3's `^[a-z][a-z0-9_]*$`; a clash falls back to `p<index>`).
//! - Control **outputs** are read-only parameters (meters), except the latency port.
//! - Plain `lv2:minimum`/`maximum`/`default` (× the sample rate for `lv2:sampleRate` ports);
//!   `lv2:toggled` → `BOOL`; `lv2:integer` → step 1; `lv2:enumeration` whose scale points are
//!   exactly `0..n` → enum labels (otherwise stepped; the labels still show as the plugin's text);
//!   `pprops:logarithmic` (with `min > 0`) → `Log` taper; `units:db` → `Db` unit + taper; other
//!   `units:` → the module API's units where they exist.
//! - `pprops:notAutomatic` → not automatable; `pprops:notOnGUI` and the plugin's own `lv2:enabled`
//!   (the rack has host bypass) and `lv2:freeWheeling` ports → `HIDDEN`.
//! - `pg:group` → a parameter group named by the group's `lv2:name`/`rdfs:label`.
//! - Degenerate ranges become hidden read-only 0..1 parameters, like CLAP's.

use std::collections::HashSet;

use vox_lv2_abi::{uri, uri_str};
use vox_module_api::{
    GroupId, LocalizedText, ParamFlags, ParamGroup, ParamId, ParamInfo, Taper, Unit,
};

/// Enumerations with more scale points than this stay plain stepped parameters.
const MAX_ENUM_LABELS: usize = 64;
/// More parameters than this: groups start collapsed.
const COLLAPSE_GROUPS_ABOVE: usize = 24;

/// What a port is, for connecting it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum PortKind {
    AudioIn,
    AudioOut,
    ControlIn,
    ControlOut,
    AtomIn,
    AtomOut,
    CvIn,
    CvOut,
    /// A port of another type marked `lv2:connectionOptional`: connected to null.
    Optional,
    /// A port of another type the plugin needs: it can't be hosted.
    #[default]
    Unsupported,
}

/// One port's description, read from the plugin's data through lilv.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct RawPort {
    pub(crate) index: u32,
    pub(crate) symbol: String,
    pub(crate) name: String,
    pub(crate) kind: PortKind,
    /// `lv2:isSideChain` (audio ports): not part of the main signal.
    pub(crate) side_chain: bool,
    pub(crate) min: Option<f64>,
    pub(crate) max: Option<f64>,
    pub(crate) default: Option<f64>,
    pub(crate) toggled: bool,
    pub(crate) integer: bool,
    pub(crate) enumeration: bool,
    pub(crate) logarithmic: bool,
    pub(crate) not_automatic: bool,
    pub(crate) not_on_gui: bool,
    pub(crate) sample_rate: bool,
    pub(crate) reports_latency: bool,
    /// `units:unit`'s URI.
    pub(crate) unit: Option<String>,
    /// `lv2:designation`'s URI.
    pub(crate) designation: Option<String>,
    /// `(value, label)` scale points, sorted by value.
    pub(crate) scale_points: Vec<(f64, String)>,
    /// The `pg:group`'s label.
    pub(crate) group: Option<String>,
    /// `rsz:minimumSize` (atom ports).
    pub(crate) minimum_size: Option<u32>,
}

impl RawPort {
    fn designated(&self, what: &std::ffi::CStr) -> bool {
        self.designation.as_deref() == Some(uri_str(what))
    }

    /// The plugin's latency report (`lv2:designation lv2:latency`, or the older
    /// `lv2:reportsLatency` property).
    pub(crate) fn is_latency(&self) -> bool {
        self.kind == PortKind::ControlOut && (self.designated(uri::LATENCY) || self.reports_latency)
    }

    /// The plugin's `lv2:freeWheeling` input (1 while rendering offline).
    pub(crate) fn is_free_wheeling(&self) -> bool {
        self.kind == PortKind::ControlIn && self.designated(uri::FREE_WHEELING)
    }

    /// Whether this port becomes a parameter.
    pub(crate) fn is_param(&self) -> bool {
        match self.kind {
            PortKind::ControlIn => true,
            PortKind::ControlOut => !self.is_latency(),
            _ => false,
        }
    }

    /// The scale point label of `value`, if any.
    pub(crate) fn label_of(&self, value: f64) -> Option<&str> {
        self.scale_points
            .iter()
            .find(|(v, _)| (*v as f32).to_bits() == (value as f32).to_bits())
            .map(|(_, l)| l.as_str())
    }
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

/// `units:` URIs → the module API's units.
fn unit_of(uri: Option<&str>) -> Unit {
    let Some(name) = uri.and_then(|u| u.strip_prefix(uri::UNITS_PREFIX)) else {
        return Unit::None;
    };
    match name {
        "db" => Unit::Db,
        "hz" => Unit::Hz,
        "ms" => Unit::Ms,
        "s" => Unit::Seconds,
        "pc" => Unit::Percent,
        "frame" => Unit::Samples,
        "khz" => Unit::Custom("kHz".into()),
        "min" => Unit::Custom("min".into()),
        "bpm" => Unit::Custom("BPM".into()),
        "cent" => Unit::Custom("ct".into()),
        "semitone12TET" => Unit::Custom("st".into()),
        "oct" => Unit::Custom("oct".into()),
        "degree" => Unit::Custom("°".into()),
        "m" | "cm" | "mm" | "km" => Unit::Custom(name.into()),
        _ => Unit::None,
    }
}

/// `symbol` as a parameter key (`None` if nothing usable is left).
fn key_of(symbol: &str) -> Option<String> {
    let mut k: String = symbol
        .chars()
        .map(|c| {
            let c = c.to_ascii_lowercase();
            if c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if k.is_empty() {
        return None;
    }
    if !k.starts_with(|c: char| c.is_ascii_lowercase()) {
        k.insert_str(0, "p_");
    }
    Some(k)
}

/// Maps `ports` to a schema; `rate` scales `lv2:sampleRate` ranges.
pub(crate) fn map(ports: &[RawPort], rate: f64) -> (Vec<ParamInfo>, Vec<ParamGroup>) {
    let mut keys = HashSet::new();
    let mut params = Vec::new();
    let mut groups: Vec<ParamGroup> = Vec::new();
    for p in ports.iter().filter(|p| p.is_param()) {
        let read_only_port = p.kind == PortKind::ControlOut;
        let scale = if p.sample_rate { rate } else { 1.0 };
        let mut min = p.min.unwrap_or(0.0) * scale;
        let mut max = p.max.unwrap_or(1.0) * scale;
        let mut flags = ParamFlags::NONE;
        let mut step = None;
        let mut enum_labels = Vec::new();
        let mut taper = Taper::Linear;
        let mut unit = unit_of(p.unit.as_deref());
        if p.toggled {
            (min, max) = (0.0, 1.0);
        }
        let degenerate = !(min.is_finite() && max.is_finite()) || min >= max;
        if degenerate {
            min = if min.is_finite() { min } else { 0.0 };
            max = min + 1.0;
            unit = Unit::None;
        }
        let default = p
            .default
            .map(|d| d * scale)
            .filter(|d| d.is_finite())
            .unwrap_or(min)
            .clamp(min, max);
        let read_only = degenerate || read_only_port;
        if read_only {
            flags |= ParamFlags::READ_ONLY;
        } else if !p.not_automatic {
            flags |= ParamFlags::AUTOMATABLE;
        }
        let bypass_like = p.designated(uri::ENABLED) || p.is_free_wheeling();
        if degenerate || p.not_on_gui || bypass_like {
            flags |= ParamFlags::HIDDEN;
        }
        if !degenerate && (p.toggled || p.integer || p.enumeration) {
            flags |= ParamFlags::STEPPED;
            step = Some(1.0);
            if p.toggled || (min.to_bits() == 0f64.to_bits() && max.to_bits() == 1f64.to_bits()) {
                flags |= ParamFlags::BOOL;
            } else if p.enumeration && min.to_bits() == 0f64.to_bits() && max.fract() == 0.0 {
                let count = max as usize + 1;
                let values: Vec<f64> = p.scale_points.iter().map(|(v, _)| *v).collect();
                let exact = count <= MAX_ENUM_LABELS
                    && values.len() == count
                    && values
                        .iter()
                        .enumerate()
                        .all(|(k, v)| (*v - k as f64).abs() < 1e-9);
                if exact {
                    enum_labels = p
                        .scale_points
                        .iter()
                        .map(|(_, l)| LocalizedText::plain(l))
                        .collect();
                    unit = Unit::None;
                }
            }
            if flags.contains(ParamFlags::BOOL) {
                unit = Unit::None;
            }
        }
        if unit.is_db() {
            taper = Taper::Db {
                neg_inf_at_min: false,
            };
        } else if p.logarithmic && min > 0.0 && step.is_none() {
            taper = Taper::Log;
        }
        let group = p
            .group
            .as_deref()
            .filter(|g| !g.trim().is_empty())
            .map(|label| match groups.iter().find(|g| g.name.text == label) {
                Some(g) => g.id,
                None => {
                    let id = GroupId(groups.len() as u32 + 1);
                    groups.push(ParamGroup {
                        id,
                        key: format!("g{}", id.0),
                        name: LocalizedText::plain(label),
                        parent: None,
                        enable_param: None,
                        collapsed_by_default: false,
                    });
                    id
                }
            });
        let mut key = key_of(&p.symbol)
            .filter(|k| !keys.contains(k))
            .unwrap_or_else(|| format!("p{}", p.index));
        while keys.contains(&key) {
            key.push('_');
        }
        keys.insert(key.clone());
        let name = if p.name.trim().is_empty() {
            p.symbol.clone()
        } else {
            p.name.clone()
        };
        params.push(ParamInfo {
            id: ParamId(p.index),
            key,
            name: LocalizedText::plain(name),
            group,
            unit,
            min,
            max,
            default,
            taper,
            step,
            enum_labels,
            decimals: if step.is_some() {
                0
            } else {
                decimals_for(max - min)
            },
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
    use vox_module_api::validate_schema;

    fn control(index: u32, symbol: &str, min: f64, max: f64, default: f64) -> RawPort {
        RawPort {
            index,
            symbol: symbol.into(),
            name: symbol.to_uppercase(),
            kind: PortKind::ControlIn,
            min: Some(min),
            max: Some(max),
            default: Some(default),
            ..RawPort::default()
        }
    }

    #[test]
    fn maps_ranges_properties_units_and_groups() {
        let db = format!("{}db", uri::UNITS_PREFIX);
        let hz = format!("{}hz", uri::UNITS_PREFIX);
        let ports = vec![
            RawPort {
                kind: PortKind::AudioIn,
                ..control(0, "in", 0.0, 1.0, 0.0)
            },
            RawPort {
                unit: Some(db),
                ..control(1, "Gain", -60.0, 24.0, 0.0)
            },
            RawPort {
                enumeration: true,
                integer: true,
                scale_points: vec![(0.0, "A".into()), (1.0, "B".into()), (2.0, "C".into())],
                group: Some("Filter".into()),
                ..control(2, "mode", 0.0, 2.0, 1.0)
            },
            RawPort {
                toggled: true,
                ..control(3, "on", 0.0, 1.0, 1.0)
            },
            RawPort {
                logarithmic: true,
                unit: Some(hz),
                group: Some("Filter".into()),
                ..control(4, "freq", 20.0, 20_000.0, 1000.0)
            },
            RawPort {
                sample_rate: true,
                ..control(5, "cut", 0.0, 0.5, 0.25)
            },
            RawPort {
                kind: PortKind::ControlOut,
                ..control(6, "meter", -60.0, 0.0, 0.0)
            },
            RawPort {
                kind: PortKind::ControlOut,
                designation: Some(uri_str(uri::LATENCY).into()),
                ..control(7, "latency", 0.0, 4096.0, 0.0)
            },
            RawPort {
                designation: Some(uri_str(uri::ENABLED).into()),
                toggled: true,
                ..control(8, "enabled", 0.0, 1.0, 1.0)
            },
            control(9, "9lives", 1.0, 1.0, 1.0),
            control(10, "gain", 0.0, 1.0, 0.5),
            RawPort {
                enumeration: true,
                scale_points: vec![(1.0, "x".into()), (4.0, "y".into())],
                ..control(11, "odd", 1.0, 4.0, 1.0)
            },
        ];
        let (params, groups) = map(&ports, 48_000.0);
        validate_schema(&params, &groups).unwrap();
        let p = |id: u32| params.iter().find(|p| p.id == ParamId(id)).unwrap();
        assert_eq!(
            params.len(),
            10,
            "audio and latency ports aren't parameters"
        );
        assert_eq!((p(1).key.as_str(), p(1).unit.clone()), ("gain", Unit::Db));
        assert!(matches!(p(1).taper, Taper::Db { .. }));
        assert!(p(1).flags.contains(ParamFlags::AUTOMATABLE));
        assert_eq!(p(2).enum_labels.len(), 3);
        assert_eq!(p(2).group, p(4).group);
        assert!(p(3).flags.contains(ParamFlags::BOOL));
        assert_eq!((p(4).taper, p(4).unit.clone()), (Taper::Log, Unit::Hz));
        assert_eq!((p(5).max, p(5).default), (24_000.0, 12_000.0));
        assert!(p(6).flags.contains(ParamFlags::READ_ONLY));
        assert!(!p(6).flags.contains(ParamFlags::AUTOMATABLE));
        assert!(p(8).flags.contains(ParamFlags::HIDDEN));
        assert!(
            p(9).flags
                .contains(ParamFlags::HIDDEN | ParamFlags::READ_ONLY)
        );
        assert_eq!(p(9).key, "p_9lives");
        assert_eq!(p(10).key, "p10", "symbol clash falls back to the index");
        assert!(p(11).enum_labels.is_empty() && p(11).flags.contains(ParamFlags::STEPPED));
        assert_eq!(ports[11].label_of(4.0), Some("y"));
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].name.text, "Filter");
    }

    #[test]
    fn port_roles() {
        let lat = RawPort {
            kind: PortKind::ControlOut,
            reports_latency: true,
            ..RawPort::default()
        };
        assert!(lat.is_latency() && !lat.is_param());
        let fw = RawPort {
            kind: PortKind::ControlIn,
            designation: Some(uri_str(uri::FREE_WHEELING).into()),
            ..RawPort::default()
        };
        assert!(fw.is_free_wheeling() && fw.is_param());
        assert_eq!(key_of("Out-L 2"), Some("out_l_2".into()));
        assert_eq!(key_of(""), None);
    }
}

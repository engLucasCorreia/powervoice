//! Parameter schema: ids, flags, tapers, units, groups, validation (ADR-005 §3).
//!
//! Text conversion (`value_to_text` / `text_to_value`) lives in `text.rs`.

use std::collections::HashSet;
use std::ops::{BitOr, BitOrAssign};

use serde::{Deserialize, Serialize};

use crate::descriptor::LocalizedText;

/// Parameter id. Stable forever and never reused after a parameter is removed. Used on the RT
/// path.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ParamId(pub u32);

/// Parameter group id.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct GroupId(pub u32);

/// One parameter of a module's schema. Values are **plain** (what the DSP means, e.g. dB, Hz),
/// never normalized.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ParamInfo {
    /// Stable forever. Never reused after a parameter is removed. Used on the RT path.
    pub id: ParamId,
    /// Stable string key `^[a-z][a-z0-9_]*$`, unique per module, e.g. `"threshold_db"`.
    /// The sidecar and presets store keys. Renaming a key requires a state migration.
    pub key: String,
    /// Display name.
    pub name: LocalizedText,
    /// Group the parameter is shown in (`None` = main section).
    pub group: Option<GroupId>,
    /// Display unit; also selects the text rules.
    pub unit: Unit,
    /// Plain minimum (finite, `< max`).
    pub min: f64,
    /// Plain maximum (finite, `> min`).
    pub max: f64,
    /// Plain default (`min <= default <= max`).
    pub default: f64,
    /// Control mapping.
    pub taper: Taper,
    /// `Some(s)`: legal values are `min + k*s` (requires [`ParamFlags::STEPPED`]).
    pub step: Option<f64>,
    /// Non-empty ⇒ enum: `min = 0`, `max = len-1`, `step = 1`, STEPPED, unit `None`.
    pub enum_labels: Vec<LocalizedText>,
    /// Decimals shown by `value_to_text` in the displayed unit (e.g. 1 ⇒ `"-6.0 dB"`).
    pub decimals: u8,
    /// Declared smoothing time the module applies to changes of this parameter
    /// (0 = takes effect exactly at the event offset). Informative for the UI and tests;
    /// the module implements it (the module owns smoothing).
    pub smoothing_ms: f32,
    /// Flags.
    pub flags: ParamFlags,
}

/// Parameter flag bit set (hand-written, no `bitflags` dependency). Serialized as `u32`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ParamFlags(u32);

impl ParamFlags {
    /// No flags.
    pub const NONE: Self = Self(0);
    /// Host may change it during processing (all user-editable params of built-ins).
    pub const AUTOMATABLE: Self = Self(1 << 0);
    /// Only discrete values (step / enum / bool).
    pub const STEPPED: Self = Self(1 << 1);
    /// On/off: min 0, max 1, step 1. Implies STEPPED.
    pub const BOOL: Self = Self(1 << 2);
    /// Output: the module reports it via `out_events`; the host never sends events for it.
    /// Not AUTOMATABLE, not saved in state.
    pub const READ_ONLY: Self = Self(1 << 3);
    /// Not shown by the generic UI (still saved in state).
    pub const HIDDEN: Self = Self(1 << 4);
    /// The module's own bypass switch (BOOL, 1.0 = bypassed).
    pub const BYPASS: Self = Self(1 << 5);

    /// True if every bit of `other` is set.
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    /// Union of both sets.
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    /// Raw bits.
    pub const fn bits(self) -> u32 {
        self.0
    }
}

impl BitOr for ParamFlags {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        self.union(rhs)
    }
}

impl BitOrAssign for ParamFlags {
    fn bitor_assign(&mut self, rhs: Self) {
        *self = self.union(rhs);
    }
}

/// Mapping plain value `v` ↔ normalized control position `t` in `[0, 1]`, plus text semantics.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind")]
#[non_exhaustive]
pub enum Taper {
    /// `t = (v - min) / (max - min)`.
    Linear,
    /// `t = ln(v / min) / ln(max / min)`. Requires `0 < min < max`. Frequencies, times, Q, ratio.
    Log,
    /// Plain value in dB (unit Db/Dbfs/Dbtp required), `t` linear in dB.
    /// `neg_inf_at_min`: the value `min` means −∞ dB (linear gain 0): `value_to_text` shows
    /// `"-inf dB"`, `text_to_value` accepts `"-inf"`/`"−∞"`; the module must treat it as silence.
    Db {
        /// `min` stands for −∞ dB.
        neg_inf_at_min: bool,
    },
}

/// Display unit.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum Unit {
    /// Unitless number.
    None,
    /// Decibels (gain).
    Db,
    /// dB relative to full scale.
    Dbfs,
    /// dB true peak.
    Dbtp,
    /// Loudness units relative to full scale.
    Lufs,
    /// Hertz.
    Hz,
    /// Milliseconds.
    Ms,
    /// Seconds.
    Seconds,
    /// Percent (plain value 50 = 50 %).
    Percent,
    /// Ratio `x:1`.
    Ratio,
    /// Samples.
    Samples,
    /// Adapter-provided unit label, display only.
    Custom(String),
}

impl Unit {
    /// True for Db, Dbfs, Dbtp (the units [`Taper::Db`] requires).
    pub fn is_db(&self) -> bool {
        matches!(self, Unit::Db | Unit::Dbfs | Unit::Dbtp)
    }
}

/// A parameter group (a section of the generic UI).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ParamGroup {
    /// Group id, unique per module.
    pub id: GroupId,
    /// Stable key, unique per module.
    pub key: String,
    /// Display name.
    pub name: LocalizedText,
    /// Parent group (nesting).
    pub parent: Option<GroupId>,
    /// BOOL param rendered as the group's header toggle (Dynamics sections, EQ bands).
    pub enable_param: Option<ParamId>,
    /// Collapsed when first shown.
    pub collapsed_by_default: bool,
}

/// A schema invariant violated (see [`ParamInfo::validate`], [`validate_schema`]).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum SchemaError {
    /// A single parameter violates an invariant.
    #[error("parameter `{key}`: {reason}")]
    InvalidParam {
        /// Parameter key.
        key: String,
        /// Which invariant.
        reason: &'static str,
    },
    /// Two parameters share an id.
    #[error("duplicate parameter id {0:?}")]
    DuplicateParamId(ParamId),
    /// Two parameters share a key.
    #[error("duplicate parameter key `{0}`")]
    DuplicateParamKey(String),
    /// Two groups share an id.
    #[error("duplicate group id {0:?}")]
    DuplicateGroupId(GroupId),
    /// Two groups share a key.
    #[error("duplicate group key `{0}`")]
    DuplicateGroupKey(String),
    /// A parameter references a group that does not exist.
    #[error("parameter `{key}` references unknown group {group:?}")]
    UnknownGroup {
        /// Parameter key.
        key: String,
        /// Missing group.
        group: GroupId,
    },
    /// A group is invalid (unknown parent, parent cycle, bad enable param).
    #[error("group `{key}`: {reason}")]
    InvalidGroup {
        /// Group key.
        key: String,
        /// Which invariant.
        reason: &'static str,
    },
}

fn is_valid_key(key: &str) -> bool {
    let mut bytes = key.bytes();
    matches!(bytes.next(), Some(b'a'..=b'z'))
        && bytes.all(|b| matches!(b, b'a'..=b'z' | b'0'..=b'9' | b'_'))
}

/// `a == b` for f64 without tripping `clippy::float_cmp` (exact comparison is intended).
fn same(a: f64, b: f64) -> bool {
    a.to_bits() == b.to_bits() || (a == 0.0 && b == 0.0)
}

impl ParamInfo {
    /// True if this is an enum parameter (non-empty `enum_labels`).
    pub fn is_enum(&self) -> bool {
        !self.enum_labels.is_empty()
    }

    /// True if values are discrete (STEPPED flag).
    pub fn is_stepped(&self) -> bool {
        self.flags.contains(ParamFlags::STEPPED)
    }

    /// Checks the invariants of ADR-005 §3:
    /// - key matches `^[a-z][a-z0-9_]*$`;
    /// - values finite, `min < max`, default in range, `smoothing_ms` finite and `>= 0`;
    /// - `Log` ⇒ `min > 0`; `Db` ⇒ unit ∈ {Db, Dbfs, Dbtp}; a dB unit requires the `Db` taper;
    /// - `step` ⇔ STEPPED (`step` finite and `> 0`); `BOOL` ⇒ `0..1`, step 1;
    ///   enum labels ⇒ `0..len-1`, step 1, unit `None`;
    /// - `BYPASS` ⇒ `BOOL`; `READ_ONLY` ⇒ not `AUTOMATABLE`.
    pub fn validate(&self) -> Result<(), SchemaError> {
        let fail = |reason: &'static str| {
            Err(SchemaError::InvalidParam {
                key: self.key.clone(),
                reason,
            })
        };
        if !is_valid_key(&self.key) {
            return fail("key must match ^[a-z][a-z0-9_]*$");
        }
        if !(self.min.is_finite() && self.max.is_finite() && self.default.is_finite()) {
            return fail("min, max and default must be finite");
        }
        if self.min >= self.max {
            return fail("min must be < max");
        }
        if self.default < self.min || self.default > self.max {
            return fail("default must be within [min, max]");
        }
        if !(self.smoothing_ms.is_finite() && self.smoothing_ms >= 0.0) {
            return fail("smoothing_ms must be finite and >= 0");
        }
        match self.taper {
            Taper::Log if self.min <= 0.0 => return fail("Log taper requires min > 0"),
            Taper::Db { .. } if !self.unit.is_db() => {
                return fail("Db taper requires unit Db, Dbfs or Dbtp");
            }
            Taper::Linear | Taper::Log if self.unit.is_db() => {
                return fail("dB units require the Db taper");
            }
            _ => {}
        }
        match self.step {
            Some(s) if !(s.is_finite() && s > 0.0) => return fail("step must be finite and > 0"),
            Some(_) if !self.is_stepped() => return fail("step requires the STEPPED flag"),
            None if self.is_stepped() => return fail("STEPPED requires a step"),
            _ => {}
        }
        let unit_step = self.step.is_some_and(|s| same(s, 1.0));
        if self.flags.contains(ParamFlags::BOOL)
            && !(same(self.min, 0.0) && same(self.max, 1.0) && unit_step)
        {
            return fail("BOOL requires min 0, max 1, step 1");
        }
        if self.is_enum() {
            let last = (self.enum_labels.len() - 1) as f64;
            if !(same(self.min, 0.0) && same(self.max, last) && unit_step) {
                return fail("enum requires min 0, max len-1, step 1");
            }
            if self.unit != Unit::None {
                return fail("enum requires unit None");
            }
        }
        if self.flags.contains(ParamFlags::BYPASS) && !self.flags.contains(ParamFlags::BOOL) {
            return fail("BYPASS requires BOOL");
        }
        if self.flags.contains(ParamFlags::READ_ONLY)
            && self.flags.contains(ParamFlags::AUTOMATABLE)
        {
            return fail("READ_ONLY excludes AUTOMATABLE");
        }
        Ok(())
    }

    /// Clamps to `[min, max]` and snaps to the step grid (`min + k*step`, never above `max`).
    /// NaN maps to `default`. RT-safe.
    pub fn clamp_quantize(&self, v: f64) -> f64 {
        if v.is_nan() {
            return self.clamp_quantize(self.default);
        }
        let v = v.clamp(self.min, self.max);
        match self.step {
            Some(s) if s > 0.0 => {
                let k_max = ((self.max - self.min) / s + 1e-9).floor();
                let k = ((v - self.min) / s).round().clamp(0.0, k_max);
                (self.min + k * s).min(self.max)
            }
            _ => v,
        }
    }

    /// Plain value → normalized position `t ∈ [0, 1]` per [`Taper`] (clamped). RT-safe.
    pub fn to_normalized(&self, v: f64) -> f64 {
        let v = if v.is_nan() { self.default } else { v }.clamp(self.min, self.max);
        let t = match self.taper {
            Taper::Log => (v / self.min).ln() / (self.max / self.min).ln(),
            Taper::Linear | Taper::Db { .. } => (v - self.min) / (self.max - self.min),
        };
        t.clamp(0.0, 1.0)
    }

    /// Normalized position → plain value (inverse of [`to_normalized`](Self::to_normalized)),
    /// then [`clamp_quantize`](Self::clamp_quantize). `t` is clamped to `[0, 1]`. RT-safe.
    pub fn from_normalized(&self, t: f64) -> f64 {
        if t.is_nan() {
            return self.clamp_quantize(self.default);
        }
        let t = t.clamp(0.0, 1.0);
        let v = match self.taper {
            Taper::Log => self.min * (self.max / self.min).powf(t),
            Taper::Linear | Taper::Db { .. } => self.min + t * (self.max - self.min),
        };
        self.clamp_quantize(v)
    }
}

/// Validates a whole schema: every [`ParamInfo::validate`], id/key uniqueness of params and
/// groups, group references (param groups, parents, no parent cycles), and that each group's
/// `enable_param` exists and is BOOL.
pub fn validate_schema(params: &[ParamInfo], groups: &[ParamGroup]) -> Result<(), SchemaError> {
    let mut ids = HashSet::new();
    let mut keys = HashSet::new();
    for p in params {
        p.validate()?;
        if !ids.insert(p.id) {
            return Err(SchemaError::DuplicateParamId(p.id));
        }
        if !keys.insert(p.key.as_str()) {
            return Err(SchemaError::DuplicateParamKey(p.key.clone()));
        }
    }
    let mut group_ids = HashSet::new();
    let mut group_keys = HashSet::new();
    for g in groups {
        if !group_ids.insert(g.id) {
            return Err(SchemaError::DuplicateGroupId(g.id));
        }
        if !group_keys.insert(g.key.as_str()) {
            return Err(SchemaError::DuplicateGroupKey(g.key.clone()));
        }
    }
    for p in params {
        if let Some(group) = p.group
            && !group_ids.contains(&group)
        {
            return Err(SchemaError::UnknownGroup {
                key: p.key.clone(),
                group,
            });
        }
    }
    let parent_of = |id: GroupId| groups.iter().find(|g| g.id == id).and_then(|g| g.parent);
    for g in groups {
        let fail = |reason: &'static str| {
            Err(SchemaError::InvalidGroup {
                key: g.key.clone(),
                reason,
            })
        };
        if let Some(parent) = g.parent {
            if !group_ids.contains(&parent) {
                return fail("unknown parent group");
            }
            // Walk up at most `groups.len()` steps; reaching `g` again means a cycle.
            let mut cur = Some(parent);
            for _ in 0..groups.len() {
                match cur {
                    Some(id) if id == g.id => return fail("parent cycle"),
                    Some(id) => cur = parent_of(id),
                    None => break,
                }
            }
        }
        if let Some(pid) = g.enable_param {
            match params.iter().find(|p| p.id == pid) {
                Some(p) if p.flags.contains(ParamFlags::BOOL) => {}
                Some(_) => return fail("enable_param must be a BOOL parameter"),
                None => return fail("enable_param references an unknown parameter"),
            }
        }
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn param(key: &str, unit: Unit, min: f64, max: f64, taper: Taper) -> ParamInfo {
        ParamInfo {
            id: ParamId(0),
            key: key.into(),
            name: LocalizedText::plain(key),
            group: None,
            unit,
            min,
            max,
            default: min,
            taper,
            step: None,
            enum_labels: Vec::new(),
            decimals: 1,
            smoothing_ms: 0.0,
            flags: ParamFlags::AUTOMATABLE,
        }
    }

    fn reason(p: &ParamInfo) -> &'static str {
        match p.validate() {
            Err(SchemaError::InvalidParam { reason, .. }) => reason,
            other => panic!("expected InvalidParam, got {other:?}"),
        }
    }

    #[test]
    fn flags_ops() {
        let f = ParamFlags::AUTOMATABLE | ParamFlags::HIDDEN;
        assert!(f.contains(ParamFlags::AUTOMATABLE));
        assert!(f.contains(ParamFlags::HIDDEN));
        assert!(!f.contains(ParamFlags::BOOL));
        let mut g = ParamFlags::NONE;
        g |= ParamFlags::BOOL;
        assert_eq!(g.bits(), 1 << 2);
        assert!(f.contains(ParamFlags::NONE));
    }

    #[test]
    fn validate_accepts_good_params() {
        param(
            "gain_db",
            Unit::Db,
            -60.0,
            24.0,
            Taper::Db {
                neg_inf_at_min: true,
            },
        )
        .validate()
        .unwrap();
        param("freq_hz", Unit::Hz, 20.0, 20_000.0, Taper::Log)
            .validate()
            .unwrap();
        let mut b = param("enabled", Unit::None, 0.0, 1.0, Taper::Linear);
        b.step = Some(1.0);
        b.flags |= ParamFlags::STEPPED | ParamFlags::BOOL;
        b.validate().unwrap();
    }

    #[test]
    fn validate_rejects_bad_params() {
        let base = param("x", Unit::None, 0.0, 1.0, Taper::Linear);
        let mut p = base.clone();
        p.key = "Bad".into();
        assert!(reason(&p).contains("key"));
        let mut p = base.clone();
        p.max = 0.0;
        assert_eq!(reason(&p), "min must be < max");
        let mut p = base.clone();
        p.default = 2.0;
        assert!(reason(&p).contains("default"));
        let mut p = base.clone();
        p.min = f64::NEG_INFINITY;
        assert!(reason(&p).contains("finite"));
        let p = param("f", Unit::Hz, 0.0, 10.0, Taper::Log);
        assert!(reason(&p).contains("Log"));
        let p = param(
            "g",
            Unit::Hz,
            0.0,
            10.0,
            Taper::Db {
                neg_inf_at_min: false,
            },
        );
        assert!(reason(&p).contains("Db taper"));
        let p = param("g", Unit::Db, 0.0, 10.0, Taper::Linear);
        assert!(reason(&p).contains("dB units"));
        let mut p = base.clone();
        p.step = Some(0.5);
        assert!(reason(&p).contains("STEPPED"));
        let mut p = base.clone();
        p.flags |= ParamFlags::STEPPED;
        assert!(reason(&p).contains("requires a step"));
        let mut p = base.clone();
        p.flags |= ParamFlags::BOOL | ParamFlags::STEPPED;
        p.step = Some(0.5);
        assert!(reason(&p).contains("BOOL"));
        let mut p = base.clone();
        p.flags |= ParamFlags::BYPASS;
        assert!(reason(&p).contains("BYPASS"));
        let mut p = base.clone();
        p.flags |= ParamFlags::READ_ONLY;
        assert!(reason(&p).contains("READ_ONLY"));
        let mut p = base.clone();
        p.enum_labels = vec![LocalizedText::plain("a"), LocalizedText::plain("b")];
        p.flags |= ParamFlags::STEPPED;
        p.step = Some(1.0);
        p.validate().unwrap();
        p.max = 2.0;
        assert!(reason(&p).contains("enum"));
    }

    #[test]
    fn schema_checks_uniqueness_and_groups() {
        let a = param("a", Unit::None, 0.0, 1.0, Taper::Linear);
        let mut b = a.clone();
        b.key = "b".into();
        assert_eq!(
            validate_schema(&[a.clone(), b.clone()], &[]),
            Err(SchemaError::DuplicateParamId(ParamId(0)))
        );
        b.id = ParamId(1);
        validate_schema(&[a.clone(), b.clone()], &[]).unwrap();
        let mut c = b.clone();
        c.id = ParamId(2);
        assert!(matches!(
            validate_schema(&[a.clone(), b.clone(), c], &[]),
            Err(SchemaError::DuplicateParamKey(_))
        ));
        b.group = Some(GroupId(7));
        assert!(matches!(
            validate_schema(&[a.clone(), b.clone()], &[]),
            Err(SchemaError::UnknownGroup { .. })
        ));
        let g = |id, parent, enable| ParamGroup {
            id: GroupId(id),
            key: format!("g{id}"),
            name: LocalizedText::plain("g"),
            parent,
            enable_param: enable,
            collapsed_by_default: false,
        };
        validate_schema(&[a.clone(), b.clone()], &[g(7, None, None)]).unwrap();
        assert!(matches!(
            validate_schema(&[a.clone(), b.clone()], &[g(7, Some(GroupId(8)), None)]),
            Err(SchemaError::InvalidGroup {
                reason: "unknown parent group",
                ..
            })
        ));
        assert!(matches!(
            validate_schema(
                &[a.clone(), b.clone()],
                &[g(7, Some(GroupId(8)), None), g(8, Some(GroupId(7)), None)]
            ),
            Err(SchemaError::InvalidGroup {
                reason: "parent cycle",
                ..
            })
        ));
        assert!(matches!(
            validate_schema(&[a.clone(), b.clone()], &[g(7, None, Some(ParamId(1)))]),
            Err(SchemaError::InvalidGroup { .. })
        ));
        let mut on = param("on", Unit::None, 0.0, 1.0, Taper::Linear);
        on.id = ParamId(3);
        on.step = Some(1.0);
        on.flags |= ParamFlags::BOOL | ParamFlags::STEPPED;
        validate_schema(&[a, b, on], &[g(7, None, Some(ParamId(3)))]).unwrap();
    }

    #[test]
    fn clamp_quantize_snaps_to_grid_below_max() {
        let mut p = param("x", Unit::None, 0.0, 1.0, Taper::Linear);
        p.flags |= ParamFlags::STEPPED;
        p.step = Some(0.3);
        assert_eq!(
            p.clamp_quantize(0.95).to_bits(),
            (0.0 + 3.0 * 0.3_f64).to_bits()
        );
        assert_eq!(p.clamp_quantize(2.0).to_bits(), (3.0 * 0.3_f64).to_bits());
        assert_eq!(p.clamp_quantize(-1.0).to_bits(), 0.0_f64.to_bits());
        assert_eq!(p.clamp_quantize(0.14).to_bits(), 0.0_f64.to_bits());
        let c = param("c", Unit::None, -1.0, 1.0, Taper::Linear);
        assert!((c.clamp_quantize(f64::NAN) + 1.0).abs() < 1e-12);
        assert!((c.clamp_quantize(f64::INFINITY) - 1.0).abs() < 1e-12);
    }

    #[test]
    fn normalized_mapping_per_taper() {
        let lin = param("x", Unit::None, -10.0, 10.0, Taper::Linear);
        assert!((lin.to_normalized(0.0) - 0.5).abs() < 1e-12);
        assert!((lin.from_normalized(0.25) + 5.0).abs() < 1e-12);
        let log = param("f", Unit::Hz, 20.0, 20_000.0, Taper::Log);
        assert!((log.to_normalized(632.455_532_033_675_9) - 0.5).abs() < 1e-9);
        assert!((log.from_normalized(0.5) - 632.455_532_033_675_9).abs() < 1e-9);
        assert!((log.from_normalized(1.0) - 20_000.0).abs() < 1e-9);
        assert!((log.to_normalized(1.0)).abs() < 1e-12, "clamped below min");
        let db = param(
            "g",
            Unit::Db,
            -60.0,
            0.0,
            Taper::Db {
                neg_inf_at_min: true,
            },
        );
        assert!((db.to_normalized(-30.0) - 0.5).abs() < 1e-12);
        for i in 0..=100 {
            let t = f64::from(i) / 100.0;
            assert!((log.to_normalized(log.from_normalized(t)) - t).abs() < 1e-9);
        }
    }
}

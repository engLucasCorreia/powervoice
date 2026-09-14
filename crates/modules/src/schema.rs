//! Small builders for the built-in modules' parameter schemas, groups, telemetry and state
//! (keeps the module files close to their spec tables).

use std::collections::BTreeMap;

use vox_module_api::{
    GroupId, LocalizedText, ModuleState, ParamFlags, ParamGroup, ParamId, ParamInfo, Taper,
    TelemetryInfo, TelemetryKind, Unit,
};

/// A continuous, automatable, ungrouped parameter named `{prefix}.param.{key}`; refine it with
/// [`ParamBuild`].
pub(crate) fn param(prefix: &str, id: u32, key: &str, name: &str) -> ParamInfo {
    ParamInfo {
        id: ParamId(id),
        key: key.into(),
        name: LocalizedText::keyed(format!("{prefix}.param.{key}"), name),
        group: None,
        unit: Unit::None,
        min: 0.0,
        max: 1.0,
        default: 0.0,
        taper: Taper::Linear,
        step: None,
        enum_labels: Vec::new(),
        decimals: 1,
        smoothing_ms: 0.0,
        flags: ParamFlags::AUTOMATABLE,
    }
}

/// Builder-style refinements of a [`ParamInfo`].
pub(crate) trait ParamBuild: Sized {
    /// Shown in `group`.
    fn in_group(self, group: u32) -> Self;
    /// `Db` taper (`neg_inf_at_min: false`).
    fn db(self, unit: Unit, min: f64, max: f64, default: f64) -> Self;
    /// `Log` taper.
    fn log(self, unit: Unit, min: f64, max: f64, default: f64) -> Self;
    /// On/off switch.
    fn toggle(self, on: bool) -> Self;
    /// Enum with `(label key, label)` pairs.
    fn choice(self, labels: &[(&str, &str)], default: f64) -> Self;
    /// Linear stepped value.
    fn stepped(self, unit: Unit, min: f64, max: f64, default: f64, step: f64) -> Self;
    /// Declared smoothing time.
    fn smoothing(self, ms: f32) -> Self;
    /// Displayed decimals.
    fn decimals(self, decimals: u8) -> Self;
    /// `Db` taper whose minimum means −∞.
    fn neg_inf_at_min(self) -> Self;
    /// Not shown by the generic UI (still saved in state).
    fn hidden(self) -> Self;
}

impl ParamBuild for ParamInfo {
    fn in_group(mut self, group: u32) -> Self {
        self.group = Some(GroupId(group));
        self
    }

    fn db(mut self, unit: Unit, min: f64, max: f64, default: f64) -> Self {
        self.unit = unit;
        self.taper = Taper::Db {
            neg_inf_at_min: false,
        };
        (self.min, self.max, self.default) = (min, max, default);
        self
    }

    fn log(mut self, unit: Unit, min: f64, max: f64, default: f64) -> Self {
        self.unit = unit;
        self.taper = Taper::Log;
        (self.min, self.max, self.default) = (min, max, default);
        self
    }

    fn toggle(mut self, on: bool) -> Self {
        self.unit = Unit::None;
        self.taper = Taper::Linear;
        (self.min, self.max) = (0.0, 1.0);
        self.default = if on { 1.0 } else { 0.0 };
        self.step = Some(1.0);
        self.decimals = 0;
        self.flags |= ParamFlags::STEPPED | ParamFlags::BOOL;
        self
    }

    fn choice(mut self, labels: &[(&str, &str)], default: f64) -> Self {
        let base = self.name.key.clone().unwrap_or_else(|| self.key.clone());
        self.enum_labels = labels
            .iter()
            .map(|(k, text)| LocalizedText::keyed(format!("{base}.{k}"), *text))
            .collect();
        self.unit = Unit::None;
        self.taper = Taper::Linear;
        (self.min, self.max, self.default) = (0.0, (labels.len() - 1) as f64, default);
        self.step = Some(1.0);
        self.decimals = 0;
        self.flags |= ParamFlags::STEPPED;
        self
    }

    fn stepped(mut self, unit: Unit, min: f64, max: f64, default: f64, step: f64) -> Self {
        self.unit = unit;
        self.taper = Taper::Linear;
        (self.min, self.max, self.default) = (min, max, default);
        self.step = Some(step);
        self.flags |= ParamFlags::STEPPED;
        self
    }

    fn smoothing(mut self, ms: f32) -> Self {
        self.smoothing_ms = ms;
        self
    }

    fn decimals(mut self, decimals: u8) -> Self {
        self.decimals = decimals;
        self
    }

    fn neg_inf_at_min(mut self) -> Self {
        self.taper = Taper::Db {
            neg_inf_at_min: true,
        };
        self
    }

    fn hidden(mut self) -> Self {
        self.flags |= ParamFlags::HIDDEN;
        self
    }
}

/// A top-level group named `{prefix}.group.{key}`.
pub(crate) fn group(
    prefix: &str,
    id: u32,
    key: &str,
    name: &str,
    enable: Option<u32>,
    collapsed: bool,
) -> ParamGroup {
    ParamGroup {
        id: GroupId(id),
        key: key.into(),
        name: LocalizedText::keyed(format!("{prefix}.group.{key}"), name),
        parent: None,
        enable_param: enable.map(ParamId),
        collapsed_by_default: collapsed,
    }
}

/// A telemetry channel named `{prefix}.telemetry.{key}`, with the unit and display range of
/// its kind (GR −60…0 dB, level −100…+6 dBFS, lamp 0…1).
pub(crate) fn telemetry(
    prefix: &str,
    id: u32,
    key: &str,
    name: &str,
    kind: TelemetryKind,
    group: Option<u32>,
) -> TelemetryInfo {
    let (unit, min, max) = match kind {
        TelemetryKind::GainReduction => (Unit::Db, -60.0, 0.0),
        TelemetryKind::Level => (Unit::Dbfs, -100.0, 6.0),
        TelemetryKind::Indicator | TelemetryKind::Value => (Unit::None, 0.0, 1.0),
    };
    TelemetryInfo {
        id,
        key: key.into(),
        name: LocalizedText::keyed(format!("{prefix}.telemetry.{key}"), name),
        unit,
        min,
        max,
        kind,
        group: group.map(GroupId),
    }
}

/// Index of `id` in `params` (RT-safe linear search).
pub(crate) fn index_of(params: &[ParamInfo], id: ParamId) -> Option<usize> {
    params.iter().position(|p| p.id == id)
}

/// Values from `state` (missing keys → default, values clamp_quantized), in `params` order.
pub(crate) fn load_values(params: &[ParamInfo], state: &ModuleState, values: &mut [f64]) {
    for (p, v) in params.iter().zip(values) {
        *v = p.clamp_quantize(state.params.get(&p.key).copied().unwrap_or(p.default));
    }
}

/// State holding every parameter value (rule S1: no blob).
pub(crate) fn save_values(
    params: &[ParamInfo],
    values: &[f64],
    format_version: u32,
) -> ModuleState {
    ModuleState {
        format_version,
        params: params
            .iter()
            .zip(values)
            .map(|(p, v)| (p.key.clone(), *v))
            .collect::<BTreeMap<_, _>>(),
        blob: None,
    }
}

/// A BOOL/enum plain value as a 0/1 weight target.
pub(crate) fn switch(v: f64) -> f64 {
    if v >= 0.5 { 1.0 } else { 0.0 }
}

/// A factory preset's state (T-406, ADR-005 §2 `ModuleFactory::presets`): every parameter at its
/// schema default, with `overrides` (key, plain value) applied on top. No blob (SPEC-016 "none
/// are defined here" / SPEC-014 §2.4 "factory presets are parameter-only"): a module whose
/// factory presets need a blob (there are none in v1) builds its own `ModuleState` instead.
///
/// Panics (debug assertion) on an unknown key: a typo here is a programming error, not a runtime
/// condition, and every built-in module's presets are exercised by its own tests.
pub(crate) fn preset_state(
    params: &[ParamInfo],
    format_version: u32,
    overrides: &[(&str, f64)],
) -> ModuleState {
    let mut values: BTreeMap<String, f64> =
        params.iter().map(|p| (p.key.clone(), p.default)).collect();
    for (key, value) in overrides {
        debug_assert!(
            values.contains_key(*key),
            "preset_state: unknown parameter key `{key}`"
        );
        values.insert((*key).to_owned(), *value);
    }
    ModuleState {
        format_version,
        params: values,
        blob: None,
    }
}

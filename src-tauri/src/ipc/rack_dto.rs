//! Rack DTOs (S3-01, SPEC-012 §2.1–§2.6, ADR-003): the rack panel's slot list and the generic
//! parameter UI's schema, mapped from `vox_engine`/`vox_rack`/`vox_module_api` domain types.
//!
//! Every number or piece of text a parameter widget needs — plain value, normalized slider
//! position, Rust-formatted display text — is computed here, once, with the same [`ParamInfo`]
//! methods the rack itself uses (`to_normalized`, `value_to_text`). The UI never formats or
//! parses a value itself (SPEC-012 §2.6).
//!
//! [`SlotStatusDto`]'s and [`RackApiError`]'s messages are pre-rendered English text, not i18n
//! keys: like `param_changed.text`, they can name a module or plugin the UI has no key for, so
//! they are shown verbatim (the same exception the spec already makes for parameter text).

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use vox_engine::{
    RackApiError, RackSlot as EngineRackSlot, RackSnapshot as EngineRackSnapshot,
    ResponseCurvePoints,
};
use vox_rack::{
    CurveHandle, LocalizedText, ModuleDescriptor, NoiseProfileStatus, ParamFlags, ParamGroup,
    ParamInfo, SlotInfo, SlotStatus, Taper, TelemetryInfo, TelemetryKind, Unit,
};

use crate::ipc::error::{IpcError, IpcErrorCode};

/// Localized display text: English fallback plus an optional i18n key (ADR-005 §2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "bindings.ts")]
pub struct LocalizedTextDto {
    pub text: String,
    pub key: Option<String>,
}

impl From<&LocalizedText> for LocalizedTextDto {
    fn from(t: &LocalizedText) -> Self {
        Self {
            text: t.text.clone(),
            key: t.key.clone(),
        }
    }
}

/// A module in the registry (`rack_list_modules`; the Add-module menu, grouped client-side by
/// `features`).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "bindings.ts")]
pub struct ModuleDescriptorDto {
    pub id: String,
    pub name: LocalizedTextDto,
    pub vendor: String,
    pub description: LocalizedTextDto,
    pub features: Vec<String>,
}

impl From<&ModuleDescriptor> for ModuleDescriptorDto {
    fn from(d: &ModuleDescriptor) -> Self {
        Self {
            id: d.id.clone(),
            name: (&d.name).into(),
            vendor: d.vendor.clone(),
            description: (&d.description).into(),
            features: d.features.clone(),
        }
    }
}

/// Display unit ([`Unit`]); `Custom` carries the adapter-provided label.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(export, export_to = "bindings.ts", rename_all = "snake_case")]
pub enum UnitDto {
    None,
    Db,
    Dbfs,
    Dbtp,
    Lufs,
    Hz,
    Ms,
    Seconds,
    Percent,
    Ratio,
    Samples,
    Custom { label: String },
}

impl From<&Unit> for UnitDto {
    fn from(u: &Unit) -> Self {
        match u {
            Unit::None => Self::None,
            Unit::Db => Self::Db,
            Unit::Dbfs => Self::Dbfs,
            Unit::Dbtp => Self::Dbtp,
            Unit::Lufs => Self::Lufs,
            Unit::Hz => Self::Hz,
            Unit::Ms => Self::Ms,
            Unit::Seconds => Self::Seconds,
            Unit::Percent => Self::Percent,
            Unit::Ratio => Self::Ratio,
            Unit::Samples => Self::Samples,
            Unit::Custom(label) => Self::Custom {
                label: label.clone(),
            },
            // Unit is #[non_exhaustive] (adapters may add units later, ADR-005 §11).
            _ => Self::Custom {
                label: String::new(),
            },
        }
    }
}

/// Control mapping ([`Taper`]).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(export, export_to = "bindings.ts", rename_all = "snake_case")]
pub enum TaperDto {
    Linear,
    Log,
    Db { neg_inf_at_min: bool },
}

impl From<&Taper> for TaperDto {
    fn from(t: &Taper) -> Self {
        match *t {
            Taper::Linear => Self::Linear,
            Taper::Log => Self::Log,
            Taper::Db { neg_inf_at_min } => Self::Db { neg_inf_at_min },
            // Taper is #[non_exhaustive]; fall back to Linear for a future variant.
            _ => Self::Linear,
        }
    }
}

/// Widget-relevant flags (SPEC-012 §2.6 "Widgets from flags"): booleans rather than a raw bit
/// set, so the UI never needs to know the bit layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "bindings.ts")]
pub struct ParamFlagsDto {
    pub automatable: bool,
    pub stepped: bool,
    pub boolean: bool,
    pub read_only: bool,
    pub hidden: bool,
    pub bypass: bool,
}

impl From<ParamFlags> for ParamFlagsDto {
    fn from(f: ParamFlags) -> Self {
        Self {
            automatable: f.contains(ParamFlags::AUTOMATABLE),
            stepped: f.contains(ParamFlags::STEPPED),
            boolean: f.contains(ParamFlags::BOOL),
            read_only: f.contains(ParamFlags::READ_ONLY),
            hidden: f.contains(ParamFlags::HIDDEN),
            bypass: f.contains(ParamFlags::BYPASS),
        }
    }
}

/// One parameter's schema (SPEC-012 §2.6). `id`/`group` are the raw ids `param_set_normalized`,
/// `param_set_text` and [`ParamGroupDto::enable_param`] address.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "bindings.ts")]
pub struct ParamInfoDto {
    pub id: u32,
    pub key: String,
    pub name: LocalizedTextDto,
    pub group: Option<u32>,
    pub unit: UnitDto,
    pub min: f64,
    pub max: f64,
    pub default: f64,
    pub taper: TaperDto,
    pub step: Option<f64>,
    pub enum_labels: Vec<LocalizedTextDto>,
    pub decimals: u8,
    pub smoothing_ms: f32,
    pub flags: ParamFlagsDto,
}

impl From<&ParamInfo> for ParamInfoDto {
    fn from(p: &ParamInfo) -> Self {
        Self {
            id: p.id.0,
            key: p.key.clone(),
            name: (&p.name).into(),
            group: p.group.map(|g| g.0),
            unit: (&p.unit).into(),
            min: p.min,
            max: p.max,
            default: p.default,
            taper: (&p.taper).into(),
            step: p.step,
            enum_labels: p.enum_labels.iter().map(Into::into).collect(),
            decimals: p.decimals,
            smoothing_ms: p.smoothing_ms,
            flags: p.flags.into(),
        }
    }
}

/// A parameter group (SPEC-012 §2.6 layout: a header toggle when `enable_param` is set, one
/// nesting level via `parent`).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "bindings.ts")]
pub struct ParamGroupDto {
    pub id: u32,
    pub key: String,
    pub name: LocalizedTextDto,
    pub parent: Option<u32>,
    pub enable_param: Option<u32>,
    pub collapsed_by_default: bool,
}

impl From<&ParamGroup> for ParamGroupDto {
    fn from(g: &ParamGroup) -> Self {
        Self {
            id: g.id.0,
            key: g.key.clone(),
            name: (&g.name).into(),
            parent: g.parent.map(|p| p.0),
            enable_param: g.enable_param.map(|p| p.0),
            collapsed_by_default: g.collapsed_by_default,
        }
    }
}

/// Current value of one parameter: plain, normalized, and Rust's own display text (SPEC-012
/// §2.6: "displayed text is always Rust's formatting").
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "bindings.ts")]
pub struct ParamValueDto {
    pub id: u32,
    pub value: f64,
    pub normalized: f64,
    pub text: String,
}

impl ParamValueDto {
    /// With the rack's display text (a plugin's own text, T-803); `None` → Rust's formatting.
    fn with_text(p: &ParamInfo, value: f64, text: Option<String>) -> Self {
        Self {
            id: p.id.0,
            value,
            normalized: p.to_normalized(value),
            text: text.unwrap_or_else(|| p.value_to_text(value)),
        }
    }
}

/// A parameter changed (`param_changed` event): the slot's current index plus its new value.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "bindings.ts")]
pub struct ParamChangedDto {
    pub slot: usize,
    pub id: u32,
    pub value: f64,
    pub normalized: f64,
    pub text: String,
}

/// The rack's total latency changed (`rack_latency` event, SPEC-012 §2.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "bindings.ts")]
pub struct RackLatencyDto {
    pub latency_samples: u32,
}

/// Slot status (SPEC-012 §2.2, §2.9; T-802: `restarting` for a sandboxed plugin whose automatic
/// restart is pending). `message` is pre-rendered English text shown verbatim (see the module
/// docs) — a plugin or module name isn't something the UI can key into i18n.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(export, export_to = "bindings.ts", rename_all = "snake_case")]
pub enum SlotStatusDto {
    Active,
    Missing {
        message: String,
        too_new: bool,
    },
    Failed {
        message: String,
    },
    Restarting {
        message: String,
    },
    /// T-803: an out-of-process plugin is being started (the slot passes dry meanwhile).
    Loading,
}

/// A slot's noise-print status (S3-06, SPEC-014 §2.5, §2.8). `None` (the outer `Option` this
/// wraps on [`RackSlotDto`]) means the module has no `NoiseProfile` extension — the NR panel
/// section only renders when this DTO is present.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "bindings.ts", rename_all = "snake_case")]
pub enum NoiseProfileStatusDto {
    None,
    Loaded,
    Unreadable,
    TooNew,
}

impl From<&NoiseProfileStatus> for NoiseProfileStatusDto {
    fn from(s: &NoiseProfileStatus) -> Self {
        match s {
            NoiseProfileStatus::None => Self::None,
            NoiseProfileStatus::Loaded => Self::Loaded,
            NoiseProfileStatus::Unreadable => Self::Unreadable,
            NoiseProfileStatus::TooNew => Self::TooNew,
        }
    }
}

/// One draggable EQ-graph node (S3-07, SPEC-015 §3 "ResponseCurve components"): the band's
/// frequency/gain/Q/enable parameter ids, and which row of `rack_response_curve`'s
/// `components_db` is its own response. `gain`/`q` are `None` for HP/LP (SPEC-015 §3 "handles").
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "bindings.ts")]
pub struct CurveHandleDto {
    pub component: usize,
    pub freq: u32,
    pub gain: Option<u32>,
    pub q: Option<u32>,
    pub enable: Option<u32>,
}

impl From<&CurveHandle> for CurveHandleDto {
    fn from(h: &CurveHandle) -> Self {
        Self {
            component: h.component,
            freq: h.freq.0,
            gain: h.gain.map(|p| p.0),
            q: h.q.map(|p| p.0),
            enable: h.enable.map(|p| p.0),
        }
    }
}

/// What a module telemetry channel's value means ([`TelemetryKind`], ADR-005 §11).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "bindings.ts", rename_all = "snake_case")]
pub enum TelemetryKindDto {
    /// Applied gain change in dB, ≤ 0 for reduction.
    GainReduction,
    Level,
    /// 0/1 lamp.
    Indicator,
    Value,
}

impl From<TelemetryKind> for TelemetryKindDto {
    fn from(k: TelemetryKind) -> Self {
        match k {
            TelemetryKind::GainReduction => Self::GainReduction,
            TelemetryKind::Level => Self::Level,
            TelemetryKind::Indicator => Self::Indicator,
            TelemetryKind::Value => Self::Value,
        }
    }
}

/// One module telemetry channel (H-03, SPEC-016 §4.12 "channel descriptions travel once, with
/// the rack state"). Its values arrive in binary `VXMT` frames (`module_telemetry_subscribe`),
/// keyed by the slot's `uid`, in [`RackSlotDto::telemetry`] order.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "bindings.ts")]
pub struct TelemetryChannelDto {
    pub id: u32,
    pub key: String,
    pub name: LocalizedTextDto,
    pub unit: UnitDto,
    /// Display range.
    pub min: f64,
    pub max: f64,
    pub kind: TelemetryKindDto,
    /// Group whose header shows the meter; `null` = the slot header.
    pub group: Option<u32>,
}

impl From<&TelemetryInfo> for TelemetryChannelDto {
    fn from(t: &TelemetryInfo) -> Self {
        Self {
            id: t.id,
            key: t.key.clone(),
            name: (&t.name).into(),
            unit: (&t.unit).into(),
            min: t.min,
            max: t.max,
            kind: t.kind.into(),
            group: t.group.map(|g| g.0),
        }
    }
}

/// `rack_response_curve`'s response (S3-07, SPEC-015 §2.6.6, lean slice: JSON — the binary
/// `VXRC` frame is hardening). `components_db` is one row per band, in `curve_handles` order.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "bindings.ts")]
pub struct ResponseCurveDto {
    pub freqs_hz: Vec<f64>,
    pub sample_rate_hz: f64,
    pub total_db: Vec<f64>,
    pub components_db: Vec<Vec<f64>>,
}

impl From<ResponseCurvePoints> for ResponseCurveDto {
    fn from(p: ResponseCurvePoints) -> Self {
        Self {
            freqs_hz: p.freqs_hz,
            sample_rate_hz: p.sample_rate_hz,
            total_db: p.total_db,
            components_db: p.components_db,
        }
    }
}

impl From<&SlotStatus> for SlotStatusDto {
    fn from(s: &SlotStatus) -> Self {
        match s {
            SlotStatus::Active => Self::Active,
            SlotStatus::Missing { message, too_new } => Self::Missing {
                message: message.clone(),
                too_new: *too_new,
            },
            SlotStatus::Failed { message } => Self::Failed {
                message: message.clone(),
            },
            SlotStatus::Restarting { message } => Self::Restarting {
                message: message.clone(),
            },
            SlotStatus::Loading => Self::Loading,
        }
    }
}

/// One rack slot: identity, status, schema and current values (`rack_get`, every mutating
/// command's result, `rack_changed`).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "bindings.ts")]
pub struct RackSlotDto {
    /// Stable within this rack's lifetime (never persisted; not the slot's position).
    pub uid: u64,
    /// `"id@version"`, or the stored reference verbatim for a placeholder.
    pub module: String,
    /// The module id alone (registry key, no `@version`); `None` for a placeholder (T-406: which
    /// preset menu — `module_presets_list`/`module_preset_save` — applies to this slot).
    pub module_id: Option<String>,
    pub name: String,
    pub bypass: bool,
    pub latency_samples: u32,
    pub status: SlotStatusDto,
    pub params: Vec<ParamInfoDto>,
    pub groups: Vec<ParamGroupDto>,
    /// Index-aligned with `params`; empty for a placeholder (no schema).
    pub values: Vec<ParamValueDto>,
    /// `Some` only for a module with the `NoiseProfile` extension (S3-06): drives the NR panel's
    /// Capture button and status line.
    pub noise_profile: Option<NoiseProfileStatusDto>,
    /// `Some` only for a module with the `ResponseCurve` extension (S3-07): the EQ graph panel
    /// only renders when this is present, generic to any future module that exposes a curve.
    pub curve_handles: Option<Vec<CurveHandleDto>>,
    /// The module's telemetry channels (H-03): empty without the `Telemetry` extension (and for
    /// placeholders). The slot header shows the `gain_reduction` channels whose `group` is `null`
    /// as meters, fed by `VXMT` frames.
    pub telemetry: Vec<TelemetryChannelDto>,
    /// The module runs out of process (a sandboxed plugin, T-802): the slot header shows its
    /// status (Running / Restarting / Failed).
    pub sandboxed: bool,
}

impl From<&EngineRackSlot> for RackSlotDto {
    fn from(s: &EngineRackSlot) -> Self {
        let info: &SlotInfo = &s.info;
        let values = info
            .params
            .iter()
            .zip(&s.values)
            .enumerate()
            .map(|(i, (p, &v))| ParamValueDto::with_text(p, v, s.texts.get(i).cloned()))
            .collect();
        Self {
            uid: info.uid.0,
            module: info.module.clone(),
            module_id: info.module_id.clone(),
            name: info.name.clone(),
            bypass: info.bypass,
            latency_samples: info.latency_samples,
            status: (&info.status).into(),
            params: info.params.iter().map(Into::into).collect(),
            groups: info.groups.iter().map(Into::into).collect(),
            values,
            noise_profile: info.noise_profile.as_ref().map(Into::into),
            curve_handles: info
                .curve_handles
                .as_ref()
                .map(|hs| hs.iter().map(Into::into).collect()),
            telemetry: info.telemetry.iter().map(Into::into).collect(),
            sandboxed: info.sandboxed,
        }
    }
}

/// The rack panel's whole state (`rack_get`, every mutating command's result, and the
/// `rack_changed` event).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "bindings.ts")]
pub struct RackStateDto {
    /// Slots, top to bottom (processing order).
    pub slots: Vec<RackSlotDto>,
    /// Whole-rack A/B state (listening only, SPEC-012 §2.3).
    pub ab: bool,
    pub latency_samples: u32,
}

impl From<&EngineRackSnapshot> for RackStateDto {
    fn from(s: &EngineRackSnapshot) -> Self {
        Self {
            slots: s.slots.iter().map(Into::into).collect(),
            ab: s.ab,
            latency_samples: s.latency_samples,
        }
    }
}

/// Maps a rack command failure to the shared IPC error shape (ADR-003). `Rack`'s message is
/// Rust-formatted `vox_rack::RackError` text (see the module docs), carried in `params.message`.
pub fn rack_ipc_error(err: RackApiError) -> IpcError {
    match err {
        RackApiError::Unavailable => {
            IpcError::new(IpcErrorCode::Internal, "error.rack_unavailable")
        }
        RackApiError::Rack(message) => {
            IpcError::new(IpcErrorCode::InvalidArgument, "error.rack_rejected")
                .with_param("message", message)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use vox_engine::{ModuleTelemetryFrame, ModuleTelemetryRecord};

    /// Writes the golden `VXMT` frame for the TS decoder test (SPEC-016 §4.12, ADR-003 §4:
    /// generated by the `export_bindings` run of `just gen-types`, checked by `just check-types`).
    #[test]
    fn export_bindings_vxmt_fixture() {
        let f = ModuleTelemetryFrame {
            seq: 5,
            frame_time_ns: 987_654_321_000,
            records: vec![
                ModuleTelemetryRecord {
                    slot_uid: 3,
                    values: vec![-6.5],
                },
                ModuleTelemetryRecord {
                    slot_uid: 12,
                    values: vec![0.0, -24.0, 1.0],
                },
            ],
        };
        let hex: String = f.encode().iter().map(|b| format!("{b:02x}")).collect();
        let ts = format!(
            "// Generated by the `export_bindings_vxmt_fixture` test (src-tauri, `just gen-types`). Do not edit.\n\
             \n\
             /** A `VXMT` frame encoded by Rust (`ModuleTelemetryFrame::encode`) for the TS decoder contract test. */\n\
             export const VXMT_FIXTURE_HEX =\n  \"{hex}\";\n\
             \n\
             /** The values encoded in `VXMT_FIXTURE_HEX`. */\n\
             export const VXMT_FIXTURE_FIELDS = {{\n  \
             seq: 5,\n  frameTimeNs: 987654321000,\n  records: [\n    \
             {{ slotUid: 3, values: [-6.5] }},\n    \
             {{ slotUid: 12, values: [0, -24, 1] }},\n  ],\n}} as const;\n",
        );
        let dir = std::env::var_os("TS_RS_EXPORT_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("bindings"));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("vxmt_fixture.ts"), ts).unwrap();
    }
}

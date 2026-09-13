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
use vox_engine::{RackApiError, RackSlot as EngineRackSlot, RackSnapshot as EngineRackSnapshot};
use vox_rack::{
    LocalizedText, ModuleDescriptor, ParamFlags, ParamGroup, ParamInfo, SlotInfo, SlotStatus,
    Taper, Unit,
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
    fn new(p: &ParamInfo, value: f64) -> Self {
        Self {
            id: p.id.0,
            value,
            normalized: p.to_normalized(value),
            text: p.value_to_text(value),
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

/// Slot status (SPEC-012 §2.2, §2.9). `message` is pre-rendered English text shown verbatim (see
/// the module docs) — a plugin or module name isn't something the UI can key into i18n.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(export, export_to = "bindings.ts", rename_all = "snake_case")]
pub enum SlotStatusDto {
    Active,
    Missing { message: String, too_new: bool },
    Failed { message: String },
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
    pub name: String,
    pub bypass: bool,
    pub latency_samples: u32,
    pub status: SlotStatusDto,
    pub params: Vec<ParamInfoDto>,
    pub groups: Vec<ParamGroupDto>,
    /// Index-aligned with `params`; empty for a placeholder (no schema).
    pub values: Vec<ParamValueDto>,
}

impl From<&EngineRackSlot> for RackSlotDto {
    fn from(s: &EngineRackSlot) -> Self {
        let info: &SlotInfo = &s.info;
        let values = info
            .params
            .iter()
            .zip(&s.values)
            .map(|(p, &v)| ParamValueDto::new(p, v))
            .collect();
        Self {
            uid: info.uid.0,
            module: info.module.clone(),
            name: info.name.clone(),
            bypass: info.bypass,
            latency_samples: info.latency_samples,
            status: (&info.status).into(),
            params: info.params.iter().map(Into::into).collect(),
            groups: info.groups.iter().map(Into::into).collect(),
            values,
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

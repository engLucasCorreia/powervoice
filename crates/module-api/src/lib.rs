//! Module API: the single contract for everything in the rack (ADR-005).
//!
//! Built-in modules, separately installed module packages and external-plugin adapters all
//! implement [`Module`]. The rack, latency compensation, presets, the sidecar and the generic
//! parameter UI only ever see this API.
//!
//! Overview:
//! - Identity: [`ModuleDescriptor`], [`ModuleRef`], [`Version`], [`ModuleFactory`] (ADR-005 §2).
//! - Parameters: [`ParamInfo`] with [`Taper`], [`Unit`], [`ParamFlags`] and locale-neutral text
//!   conversion, [`ParamGroup`], [`validate_schema`] (§3).
//! - Sample-accurate parameter events: [`ParamEvent`], [`EventList`], [`OutputEvents`],
//!   [`segments`] (§4).
//! - Lifecycle and processing: [`ActivateConfig`], [`ProcessContext`], [`ProcessMode`], [`Tail`]
//!   (§5–§6).
//! - State: [`ModuleState`], [`prepare_state`] (§10).
//! - Packaged modules (ADR-006 §2): [`ModuleInfo`], the JSON a CLAP-packaged module publishes
//!   through [`MODULE_INFO_EXTENSION_ID`]; [`is_valid_module_id`].
//! - Extensions: [`ExtensionId`], [`Extension`], [`Telemetry`], [`ResponseCurve`],
//!   [`NoiseProfile`], [`AdapterHealth`], [`ParamText`], [`PluginEditor`] (§11; the last three
//!   are host-internal, T-802/T-803/T-901).
//!
//! Feature `test-util` adds [`test_util::ModuleTestHost`] and the reference module
//! [`test_util::TestGain`].
//!
//! Real-time rules: [`Module::process`] and [`Module::reset`] never allocate, lock, block, log or
//! panic on valid input. Everything in this crate that is documented as "RT-safe" follows the
//! same rules.

mod descriptor;
mod event;
mod extension;
mod info;
mod module;
mod param;
mod process;
mod state;
mod text;

#[cfg(feature = "test-util")]
pub mod test_util;

pub use descriptor::{
    LocalizedText, MODULE_API_VERSION, ModuleDescriptor, ModuleFactory, ModulePreset, ModuleRef,
    ParseModuleRefError, ParseVersionError, Version, features, is_valid_module_id,
};
pub use event::{
    DEFAULT_EVENT_CAPACITY, EventList, EventListError, OutputEvents, ParamEvent, Segment, segments,
};
pub use extension::{
    AdapterHealth, AtomicF32, CurveBranch, CurveHandle, EditorRequest, EditorUpdate, Extension,
    ExtensionId, Hold, NoiseProfile, ParamText, PluginEditor, ResponseCurve, Telemetry,
    TelemetryCells, TelemetryInfo, TelemetryKind, TransferCurve, TransferHandle, adapter_health,
    noise_profile, param_text, plugin_editor, response_curve, telemetry, transfer_curve,
};
pub use info::{MODULE_INFO_EXTENSION_ID, ModuleInfo, ModuleInfoError};
pub use module::{Module, ModuleError};
pub use param::{
    GroupId, ParamFlags, ParamGroup, ParamId, ParamInfo, SchemaError, Taper, Unit, validate_schema,
};
pub use process::{
    ActivateConfig, ChannelLayout, HostRequest, ProcessContext, ProcessMode, ProcessStatus, Tail,
    Transport,
};
pub use state::{ModuleState, StateError, prepare_state};
pub use text::{TEXT_KEY_OFF, TEXT_KEY_ON};

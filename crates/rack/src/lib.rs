//! Pure chain host (ADR-001 §4): module registry, chains, live chain swap with crossfades,
//! host bypass and whole-rack A/B, parameter routing with coalescing, the RT drain of module
//! reports, the non-finite guard and offline render — shared by engine, export, bake and the
//! CLI (SPEC-012).
//!
//! # Pieces
//! - [`Registry`]: module id → factory, one version per id; the composition roots register the
//!   built-ins (`vox_modules::builtin_factories`).
//! - [`RackModel`] / [`SlotModel`]: the rack as plain data, in the sidecar's slot schema
//!   (ADR-005 §10). Unknown modules become **placeholders** (dry, latency 0, written back
//!   verbatim).
//! - [`Chain`]: the slots processed in series — the one rack code path (ADR-001 §5).
//! - [`RackHost`] (control thread) and [`LiveRack`] (audio thread): the live rack.
//! - [`offline::render`]: offline mode, 4096-frame blocks, latency trim.
//!
//! # Live edits (ADR-002 §3, ADR-005 §7/§12)
//! The control thread ([`RackHost`]) owns the authoritative model and a parameter mirror. For an
//! insert, removal, reorder or replacement it builds and activates a **new** chain off the audio
//! thread and sends it through a lock-free command ring. The chain carries a *plan*: for every
//! slot the module instance of an **untouched** slot is **moved** from the live chain at the swap
//! (pointer moves on the audio thread: no allocation, no `activate`, its state — delay lines,
//! envelopes — stays warm), while inserted or replaced slots bring fresh instances. Each edit is
//! a per-slot 15 ms linear equal-gain crossfade: an inserted slot fades in from its undelayed
//! input, a removed slot fades out to it (then turns into a pass-through), a replaced instance
//! crossfades to its successor. The retired chain — shells, dead slots, finished outgoing
//! instances — goes back through the **return ring** and is deactivated and dropped by the
//! control thread; nothing is ever dropped on the audio thread.
//!
//! RT → control: module output events (READ_ONLY reports), restart requests, failures and
//! finished transitions travel through the RT event ring ([`RackEvent`]).

mod chain;
mod delay;
mod host;
mod live;
mod mix;
mod model;
pub mod offline;
mod registry;
mod shim;
mod slot;

pub use chain::Chain;
pub use host::{NoiseProfileStatus, RackHost, RackNotice, SlotInfo, SlotStatus, SlotTelemetry};
pub use live::{LiveRack, RackCommand};
pub use mix::xfade_gain;
pub use model::{RackModel, SlotModel};
pub use registry::{Registry, RegistryError, Resolved};
pub use shim::DualMonoShim;
/// Module-API types a rack driver needs (the engine drives the rack through these without a
/// direct `module-api` edge, ADR-001 §2). `NoiseProfile` (S3-06): the noise-print capture
/// extension a target slot's [`RackHost::noise_profile_extension`] returns. `ResponseCurve` /
/// `CurveHandle` (S3-07): the EQ-graph extension a target slot's
/// [`RackHost::response_curve_extension`] returns. `Telemetry` / `TelemetryInfo` /
/// `TelemetryKind` (H-03): the per-slot meter channels ([`SlotInfo::telemetry`],
/// [`RackHost::read_telemetry`]).
pub use vox_module_api::{
    ActivateConfig, ChannelLayout, CurveHandle, GroupId, LocalizedText, ModuleDescriptor,
    ModuleError, ModuleFactory, NoiseProfile, ParamFlags, ParamGroup, ParamId, ParamInfo,
    ProcessMode, ResponseCurve, Taper, Telemetry, TelemetryInfo, TelemetryKind, Transport, Unit,
};

use vox_module_api::{DEFAULT_EVENT_CAPACITY, EventListError, SchemaError, StateError};

#[cfg(test)]
vox_module_api::install_test_allocator!();

/// Bypass / swap / replacement / A/B crossfade (SPEC-012 §3 `xfade_ms`).
pub const XFADE_MS: f64 = 15.0;
/// Slots per rack (SPEC-012 §3 `max_slots`).
pub const MAX_SLOTS: usize = 16;
/// Realtime sub-block (SPEC-000 `MAX_BLOCK`): the `max_block` of live chains.
pub const MAX_BLOCK: u32 = 1024;
/// Offline render block (SPEC-000 `OFFLINE_BLOCK`).
pub const OFFLINE_BLOCK: u32 = 4096;
/// Capacity of the control → audio command ring (ADR-002 §2).
pub const COMMAND_RING_CAPACITY: usize = 1024;
/// Capacity of the audio → control RT event ring (ADR-002 §2).
pub const EVENT_RING_CAPACITY: usize = 1024;
/// Capacity of the return ring for retired chains (ADR-002 §2).
pub const RETURN_RING_CAPACITY: usize = 64;
/// Chain swaps the control thread keeps in flight at most (ADR-002 §2).
pub const MAX_SWAPS_IN_FLIGHT: usize = 32;
/// Commands the live rack drains per `process` call at most (ADR-002 §3).
pub const MAX_COMMANDS_PER_CALL: usize = 256;

/// Crossfade length in samples at `sample_rate` (`round(15 ms × rate)`, at least 1).
pub fn xfade_samples(sample_rate: f64) -> u32 {
    ((sample_rate * XFADE_MS / 1000.0).round() as u32).max(1)
}

/// Stable identity of a slot within one rack (not persisted). Commands and RT events address
/// slots by identity, so they stay correct across chain swaps.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SlotUid(pub u64);

/// Why the rack bypassed a slot on its own (SPEC-012 §2.9).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FailReason {
    /// The module produced a non-finite sample.
    NonFinite,
    /// The module returned `ProcessStatus::Error` (adapters only).
    ModuleError,
}

/// Audio → control events (`Copy`, carried by the RT event ring).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RackEvent {
    /// A module reported a value through `out_events` (READ_ONLY parameters, adapter-originated
    /// changes).
    ParamReport {
        /// Slot.
        slot: SlotUid,
        /// Parameter.
        id: ParamId,
        /// Reported plain value.
        value: f64,
    },
    /// The module requested `HostRequest::Restart` (reported once per instance).
    RestartRequested {
        /// Slot.
        slot: SlotUid,
    },
    /// The slot failed and was bypassed (15 ms crossfade).
    SlotFailed {
        /// Slot.
        slot: SlotUid,
        /// Why.
        reason: FailReason,
    },
    /// A removed slot finished fading out and is now a pass-through (compaction may follow).
    SlotRemoved {
        /// Slot.
        slot: SlotUid,
    },
    /// A replacement crossfade finished; the outgoing instance can be retired.
    ReplaceDone {
        /// Slot.
        slot: SlotUid,
    },
}

/// Capacities of the per-slot event storage.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RackOptions {
    /// Events one `process()` call of a module receives at most (ADR-002: 512).
    pub event_capacity: usize,
    /// Events queued per slot for upcoming blocks (including carried-over ones).
    pub queue_capacity: usize,
}

impl Default for RackOptions {
    fn default() -> Self {
        Self {
            event_capacity: DEFAULT_EVENT_CAPACITY,
            queue_capacity: 2 * DEFAULT_EVENT_CAPACITY,
        }
    }
}

/// Error from a control-thread rack operation.
#[derive(Debug, thiserror::Error)]
pub enum RackError {
    /// Slot index out of range.
    #[error("slot index {index} out of range (rack has {len} slots)")]
    IndexOutOfRange {
        /// Requested index.
        index: usize,
        /// Number of slots.
        len: usize,
    },
    /// The rack already holds [`MAX_SLOTS`] slots.
    #[error("a rack holds at most {MAX_SLOTS} slots")]
    TooManySlots,
    /// The module has no mono or dual-mono-capable layout.
    #[error("{name} has an unsupported channel layout")]
    UnsupportedLayout {
        /// Module id.
        id: String,
        /// Module display name.
        name: String,
    },
    /// The module's parameter schema is invalid.
    #[error("module `{id}`: invalid parameter schema: {source}")]
    Schema {
        /// Module id.
        id: String,
        /// Violated invariant.
        source: SchemaError,
    },
    /// The activation config is invalid.
    #[error("invalid rack configuration: {0}")]
    InvalidConfig(&'static str),
    /// `activate` on an active chain.
    #[error("rack is already active")]
    AlreadyActive,
    /// A module failed to activate (it is not inserted).
    #[error("Couldn't start {name}: {source}")]
    Activate {
        /// Slot index.
        index: usize,
        /// Module id.
        id: String,
        /// Module display name.
        name: String,
        /// Module error.
        source: ModuleError,
    },
    /// The factory failed to create an instance.
    #[error("Couldn't start {id}: {source}")]
    Create {
        /// Module id.
        id: String,
        /// Factory error.
        source: ModuleError,
    },
    /// The slot's stored state could not be loaded.
    #[error("module `{id}`: invalid state: {message}")]
    InvalidState {
        /// Module id.
        id: String,
        /// What went wrong.
        message: String,
    },
    /// The state pipeline failed.
    #[error("module `{id}`: {source}")]
    State {
        /// Module id.
        id: String,
        /// State error.
        source: StateError,
    },
    /// No module with that id is registered.
    #[error("unknown module `{0}`")]
    UnknownModule(String),
    /// The slot is a placeholder (its module is missing).
    #[error("slot {index} is a placeholder")]
    NotLoaded {
        /// Slot index.
        index: usize,
    },
    /// The module has no such parameter (or it is READ_ONLY).
    #[error("slot {index}: no writable parameter {id:?}")]
    UnknownParam {
        /// Slot index.
        index: usize,
        /// Parameter id.
        id: ParamId,
    },
    /// Typed text did not parse (nothing was sent).
    #[error("`{0}` is not a valid value")]
    InvalidText(String),
}

/// Error from [`Chain::push_event`] (RT-safe, `Copy`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PushEventError {
    /// No slot at that index.
    #[error("no slot at index {0}")]
    NoSuchSlot(usize),
    /// Queue full or event out of order.
    #[error(transparent)]
    List(#[from] EventListError),
}

//! UI-facing rack API (S3-01, SPEC-012 §2.1–§2.6): the commands the app sends and the read-only
//! snapshot it renders. Thin: everything but assembling a [`RackSnapshot`] from a live
//! [`RackHost`] lives in `vox_rack`; this module never touches audio.
//!
//! The live rack only exists alongside an open output stream (ADR-002 §5: `RackHost` is rebuilt
//! at [`ActivateConfig`](vox_rack::ActivateConfig) whenever the stream (re)opens). Without one,
//! commands and snapshots report [`RackApiError::Unavailable`] / an empty snapshot: on the
//! owner's machine an output device is open essentially always (PipeWire default), so this is a
//! narrow edge case, not the common path.

use vox_rack::{ParamId, RackHost, SlotInfo};

/// A rack edit or parameter change from the UI (SPEC-012 §2.1–§2.4). Slots are addressed by
/// their current index in the rack (not the internal [`SlotUid`](vox_rack::SlotUid), which never
/// reaches the UI).
#[derive(Clone, Debug)]
pub enum RackCommand {
    /// Adds a registered module at `index` (`0..=len`).
    Add {
        /// Module id (registry key, no `@version`).
        module_id: String,
        /// Insertion index.
        index: usize,
    },
    /// Removes the slot at `index`.
    Remove {
        /// Slot index.
        index: usize,
    },
    /// Moves the slot at `from` to `to` (drag-reorder).
    Move {
        /// Source index.
        from: usize,
        /// Destination index.
        to: usize,
    },
    /// Per-slot bypass toggle.
    SetBypass {
        /// Slot index.
        index: usize,
        /// New bypass state.
        on: bool,
    },
    /// Whole-rack A/B (listening only).
    SetAb {
        /// New state.
        on: bool,
    },
    /// Sets a parameter from a normalized `[0, 1]` slider position.
    SetParamNormalized {
        /// Slot index.
        index: usize,
        /// Parameter id.
        id: ParamId,
        /// Normalized position.
        value: f64,
    },
    /// Sets a parameter from typed text (Rust parses it; unparseable text is rejected).
    SetParamText {
        /// Slot index.
        index: usize,
        /// Parameter id.
        id: ParamId,
        /// Typed text.
        text: String,
    },
    /// Restarts the slot's instance from its committed state (Restart of a failed slot, or a
    /// manual retry).
    Restart {
        /// Slot index.
        index: usize,
    },
}

/// One slot's schema/status plus its current mirrored values (index-aligned with
/// `info.params`; empty for placeholders, which have no schema).
#[derive(Clone, Debug)]
pub struct RackSlot {
    /// Identity, module reference, name, bypass, latency, status and parameter schema.
    pub info: SlotInfo,
    /// Current plain value of each parameter in `info.params`.
    pub values: Vec<f64>,
}

/// The rack as the UI renders it (`rack_get`, every mutating command's result, and the
/// `rack_changed` event after a restart or failure, SPEC-012 §2.1).
#[derive(Clone, Debug, Default)]
pub struct RackSnapshot {
    /// Slots, top to bottom (processing order).
    pub slots: Vec<RackSlot>,
    /// Whole-rack A/B state (listening only).
    pub ab: bool,
    /// Total latency in samples (sum of slot latencies, bypassed included, SPEC-012 §2.5).
    pub latency_samples: u32,
}

impl RackSnapshot {
    /// Reads the current state of `host` (control thread only).
    pub(crate) fn read(host: &RackHost) -> Self {
        let slots = (0..host.len())
            .filter_map(|i| {
                let info = host.slot_info(i)?;
                let values = info
                    .params
                    .iter()
                    .map(|p| host.param_value(i, p.id).unwrap_or(p.default))
                    .collect();
                Some(RackSlot { info, values })
            })
            .collect();
        Self {
            slots,
            ab: host.ab(),
            latency_samples: host.total_latency_samples(),
        }
    }
}

/// Error from a rack command.
#[derive(Clone, Debug)]
pub enum RackApiError {
    /// No output stream is open, so there is no live rack to command (see the module docs).
    Unavailable,
    /// The rack rejected the command (`vox_rack::RackError`'s message).
    Rack(String),
}

impl std::fmt::Display for RackApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unavailable => write!(f, "no audio output is open"),
            Self::Rack(msg) => write!(f, "{msg}"),
        }
    }
}

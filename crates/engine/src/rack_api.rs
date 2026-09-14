//! UI-facing rack API (S3-01, SPEC-012 §2.1–§2.6): the commands the app sends and the read-only
//! snapshot it renders. Thin: everything but assembling a [`RackSnapshot`] from a live
//! [`RackHost`] lives in `vox_rack`; this module never touches audio.
//!
//! The live rack only exists alongside an open output stream (ADR-002 §5: `RackHost` is rebuilt
//! at [`ActivateConfig`](vox_rack::ActivateConfig) whenever the stream (re)opens). Without one,
//! commands and snapshots report [`RackApiError::Unavailable`] / an empty snapshot: on the
//! owner's machine an output device is open essentially always (PipeWire default), so this is a
//! narrow edge case, not the common path.

use std::sync::Arc;

use vox_rack::{ModuleState, NoiseProfile, ParamId, RackHost, RackModel, SlotInfo};

/// Cap on [`EngineHandle::response_curve`](crate::EngineHandle::response_curve)'s frequency list
/// (S3-07, SPEC-015 §2.6.6 "≤ 512 points"): an oversized request is truncated, not rejected — the
/// graph never legitimately needs more than one point per device-pixel column.
pub const MAX_RESPONSE_CURVE_POINTS: usize = 512;

/// The built-in Noise Reduction module's id (SPEC-014 §2.1, ADR-005 §2). Mirrors
/// `vox_modules::NoiseReduction::ID`; `vox-engine` doesn't otherwise depend on `vox-modules`
/// (composition roots register modules, not the engine crate), so it is repeated here rather than
/// imported — `nr_capture_module_id_matches_the_noise_reduction_module` guards against drift.
pub const NOISE_REDUCTION_MODULE_ID: &str = "org.powervoice.noise-reduction";

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
    /// Sets a parameter from a plain value (S3-07, SPEC-015 §2.6.6): the EQ graph's draggable
    /// nodes send Hz/dB/Q values directly, through the inverse of the display axis mapping —
    /// not taper code, so the UI still never runs filter math. Clamp-quantized and mirrored the
    /// same way as [`RackCommand::SetParamNormalized`]/[`RackCommand::SetParamText`].
    SetParamPlain {
        /// Slot index.
        index: usize,
        /// Parameter id.
        id: ParamId,
        /// Plain value.
        value: f64,
    },
    /// Restarts the slot's instance from its committed state (Restart of a failed slot, or a
    /// manual retry).
    Restart {
        /// Slot index.
        index: usize,
    },
    /// Applies a module preset to slot `index` (T-406, SPEC-012 §2.7, ADR-005 §12): a state with
    /// a blob (e.g. a Noise Reduction preset that "includes the print", SPEC-014 §2.4) replaces
    /// the instance behind the 15 ms crossfade; a parameter-only state is smoothed, like typed
    /// values, with no restart, and leaves the slot's own committed blob untouched.
    ApplyModulePreset {
        /// Slot index.
        index: usize,
        /// The preset's module state.
        state: ModuleState,
    },
    /// Resets every writable parameter of slot `index` to its schema default (T-406): the same
    /// smoothed, no-restart path as [`RackCommand::ApplyModulePreset`] with a parameter-only
    /// state — a committed blob (a noise print) is kept.
    ResetToDefault {
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
    /// Display text of each value (index-aligned with `values`): the plugin's own text for a
    /// plugin that formats its values (T-803), else the module API's text rules.
    pub texts: Vec<String>,
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
                let texts = host.param_texts(i);
                Some(RackSlot {
                    info,
                    values,
                    texts,
                })
            })
            .collect();
        Self {
            slots,
            ab: host.ab(),
            latency_samples: host.total_latency_samples(),
        }
    }
}

/// What [`crate::EngineHandle::nr_capture_prepare`] resolves before a Capture Noise Print job
/// reads any audio (SPEC-014 §2.3): the target slot, the committed state of the slots before it
/// (for the offline pre-render), the slot's current parameter values (`NoiseProfile::capture`'s
/// `values` argument) and the extension handle itself — safe to call `capture` on from the job's
/// own worker thread (module docs; ADR-002).
#[derive(Clone)]
pub struct NrCapturePrep {
    /// The target slot's current index.
    pub index: usize,
    /// A Noise Reduction slot didn't exist and was inserted as the first slot.
    pub inserted: bool,
    /// The committed state of every slot before `index` (placeholders omitted).
    pub upstream_model: RackModel,
    /// The target slot's current plain parameter values (index-aligned with its schema).
    pub values: Vec<f64>,
    /// The target slot's `NoiseProfile` handle.
    pub extension: Arc<dyn NoiseProfile>,
}

/// What [`crate::EngineHandle::response_curve`] returns (S3-07, SPEC-015 §2.6.6, lean slice: a
/// plain in-memory answer, not yet the binary `VXRC` frame — hardening). Evaluated on the
/// control thread from the target slot's `ResponseCurve` extension at the **target** values of
/// the parameter mirror (so it reflects a `set_param_plain`/`SetParamPlain` echo immediately,
/// with no audio processed in between), at the rack's rate.
#[derive(Clone, Debug)]
pub struct ResponseCurvePoints {
    /// The requested frequencies (Hz), unchanged and in the order given.
    pub freqs_hz: Vec<f64>,
    /// The rate the curve was evaluated at.
    pub sample_rate_hz: f64,
    /// Total response (dB) at each of `freqs_hz`.
    pub total_db: Vec<f64>,
    /// One row per component (band), in the module's `handles()` order; empty when the module
    /// reports zero components.
    pub components_db: Vec<Vec<f64>>,
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

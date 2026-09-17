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

/// Cap on [`EngineHandle::transfer_curve`](crate::EngineHandle::transfer_curve)'s point count
/// (H-63, SPEC-016 §4.12 "points ≤ 1024"). An oversized request is clamped, not rejected.
pub const MAX_TRANSFER_CURVE_POINTS: usize = 1024;

/// Header length of a `VXTC` frame (SPEC-016 §4.12, version 1).
pub const VXTC_HEADER_LEN: usize = 40;

/// `VXTC` flag bit 0: the frame carries a Falling branch after the Rising one.
pub const VXTC_HAS_FALLING: u32 = 1;

/// `VXTC` handle flag bit 0: the handle's section is enabled.
pub const VXTC_HANDLE_ENABLED: u32 = 1;

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
    /// Closes slot `index`'s plugin window (T-901; no-op without one). Opening one is
    /// [`crate::EngineHandle::rack_open_editor`] (it waits for the plugin's GUI off the control
    /// thread).
    CloseEditor {
        /// Slot index.
        index: usize,
    },
    /// Closes every plugin window (T-901: document close).
    CloseAllEditors,
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

/// One draggable threshold handle of a [`TransferCurvePoints`] (SPEC-016 §4.11/§4.12), already
/// placed on the graph's x axis.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TransferCurveHandle {
    /// The component (section) the handle belongs to.
    pub component: usize,
    /// The threshold parameter a drag writes (plain dB).
    pub param: ParamId,
    /// Graph x position (dBFS) = the parameter's target value + `offset_db`.
    pub x_dbfs: f64,
    /// The detector offset applied (+3.0103 dB for an RMS-detected section, else 0).
    pub offset_db: f64,
    /// The section is enabled (handles of disabled sections are not drawn, SPEC-016 §2.6).
    pub enabled: bool,
}

/// What [`crate::EngineHandle::transfer_curve`] returns (H-63, SPEC-016 §4.11). Evaluated on the
/// control thread from the target slot's `TransferCurve` extension at the **target** values of
/// the parameter mirror, so it reflects a `SetParamPlain` echo immediately. [`Self::encode`]
/// frames it as `VXTC` for the UI (H-77, SPEC-016 §4.12).
#[derive(Clone, Debug)]
pub struct TransferCurvePoints {
    /// Input peak levels (dBFS), `points` values evenly spaced over the requested range.
    pub in_dbfs: Vec<f64>,
    /// Output peak level (dBFS) on the Rising branch; −∞ where the module mutes, never NaN.
    pub rising_db: Vec<f64>,
    /// The Falling branch, only when the module reports hysteresis for these values.
    pub falling_db: Option<Vec<f64>>,
    /// One row per component (section), in the module's order; empty when it reports none.
    /// Gains in dB, Rising branch; −∞ where the component mutes, never NaN.
    pub components_db: Vec<Vec<f64>>,
    /// The draggable threshold handles, in the module's `handles()` order.
    pub handles: Vec<TransferCurveHandle>,
}

impl TransferCurvePoints {
    /// Little-endian `VXTC` v1 encoding (H-77, SPEC-016 §4.12): a 40-byte header (`"VXTC"`,
    /// version 1, header_len 40, `seq` echoed from the request, flags
    /// ([`VXTC_HAS_FALLING`]), x_min_db, x_max_db, point count P, component count C, handle
    /// count K, reserved 0), then `f32[P]` Rising, `f32[P]` Falling when `HAS_FALLING`,
    /// `f32[C·P]` component gains (component-major) and K × `{u32 param_id, f32 x_dbfs,
    /// f32 offset_db, u32 flags}`. Levels are `f32`: −∞ is allowed (digital silence), NaN is
    /// not — a non-finite positive value would be a module bug and is written as −∞.
    pub fn encode(&self, seq: u32) -> Vec<u8> {
        let points = self.in_dbfs.len();
        let components = self.components_db.len();
        let handles = self.handles.len();
        let branches = 1 + usize::from(self.falling_db.is_some());
        let mut b = Vec::with_capacity(
            VXTC_HEADER_LEN + 4 * points * (branches + components) + 16 * handles,
        );
        let flags = if self.falling_db.is_some() {
            VXTC_HAS_FALLING
        } else {
            0
        };
        let x_min = self.in_dbfs.first().copied().unwrap_or(0.0) as f32;
        let x_max = self.in_dbfs.last().copied().unwrap_or(0.0) as f32;
        b.extend_from_slice(b"VXTC");
        b.extend_from_slice(&1u16.to_le_bytes());
        b.extend_from_slice(&(VXTC_HEADER_LEN as u16).to_le_bytes());
        b.extend_from_slice(&seq.to_le_bytes());
        b.extend_from_slice(&flags.to_le_bytes());
        b.extend_from_slice(&x_min.to_le_bytes());
        b.extend_from_slice(&x_max.to_le_bytes());
        for n in [points, components, handles] {
            b.extend_from_slice(&u32::try_from(n).unwrap_or(u32::MAX).to_le_bytes());
        }
        b.extend_from_slice(&0u32.to_le_bytes());
        let mut levels = |v: &[f64]| {
            for &x in v.iter().take(points) {
                b.extend_from_slice(&level_f32(x).to_le_bytes());
            }
            for _ in v.len()..points {
                b.extend_from_slice(&f32::NEG_INFINITY.to_le_bytes());
            }
        };
        levels(&self.rising_db);
        if let Some(falling) = &self.falling_db {
            levels(falling);
        }
        for row in &self.components_db {
            levels(row);
        }
        for h in &self.handles {
            b.extend_from_slice(&h.param.0.to_le_bytes());
            b.extend_from_slice(&(h.x_dbfs as f32).to_le_bytes());
            b.extend_from_slice(&(h.offset_db as f32).to_le_bytes());
            b.extend_from_slice(&u32::from(h.enabled).to_le_bytes());
        }
        b
    }
}

/// One `VXTC` level as `f32`: NaN (a module bug) becomes −∞ so the UI draws a gap rather than a
/// spike (SPEC-016 §4.12 "−inf allowed, never NaN").
fn level_f32(db: f64) -> f32 {
    if db.is_nan() {
        f32::NEG_INFINITY
    } else {
        db as f32
    }
}

/// Error from a rack command.
#[derive(Clone, Debug)]
pub enum RackApiError {
    /// No output stream is open, so there is no live rack to command (see the module docs).
    Unavailable,
    /// The rack rejected the command (`vox_rack::RackError`'s message).
    Rack(String),
    /// The target slot's module doesn't answer the extension the command needs (H-77,
    /// SPEC-016 §4.12 `no_extension`: e.g. a transfer curve asked of a Gain slot).
    NoExtension,
    /// The plugin couldn't open its window (T-901; the plugin's or sandbox's reason).
    Editor(String),
}

impl std::fmt::Display for RackApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unavailable => write!(f, "no audio output is open"),
            Self::Rack(msg) => write!(f, "{msg}"),
            Self::NoExtension => write!(f, "the module has no such extension"),
            Self::Editor(msg) => write!(f, "couldn't open the plugin window: {msg}"),
        }
    }
}

//! Spike-only DTOs. These are plain JSON (small, one-shot metadata — not bulk data, so ADR-003's
//! "binary for bulk" rule doesn't apply) and deliberately do **not** derive `ts_rs::TS`: they must
//! never touch `ui/src/lib/ipc/bindings.ts` or `just check-types`, which stay scoped to the
//! production DTOs in `crate::ipc`. The matching hand-written TS shapes live in
//! `ui/src/spike/types.ts`.

use serde::{Deserialize, Serialize};

/// What the spike UI should do, derived from the process environment of the Tauri backend (the
/// only place that can see how `just spike` launched it). The command that returns this
/// (`spike_env`) only exists when the `spike` feature is enabled, so a normal production build
/// simply has no such command — the UI treats a failed `invoke("spike_env")` as "not a spike
/// build" and never loads `ui/src/spike/*`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpikeEnv {
    /// `VOXEDIT_SPIKE=1`: run the automated measurement suite on load.
    pub auto_run: bool,
    /// `VOXEDIT_SPIKE_EXIT=1`: close the app once results are written (automated runs only —
    /// without this the window stays open for the owner's manual input checks).
    pub exit_after: bool,
    /// `WEBKIT_DISABLE_DMABUF_RENDERER=1`, recorded alongside results per run (ADR-009 asks for
    /// both configurations).
    pub webkit_dmabuf_disabled: bool,
}

/// Returned by `spike_waveform_peaks` alongside the binary frame sent over its `Channel`. The
/// `SPST` frame from `spike_spectrogram_texture` has no equivalent metadata return: its header
/// already carries width/height (ADR-003-style self-describing frames), so the JS side decodes
/// those from the frame itself instead of a second round trip.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WaveformMeta {
    pub total_samples: u64,
    pub sample_rate_hz: u32,
    pub spp: u32,
    pub count: u32,
    pub generation_ms: f64,
    pub frame_bytes: u32,
}

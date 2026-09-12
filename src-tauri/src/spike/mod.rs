//! T-007 platform spike (ADR-009): renderer + IPC throughput measurements.
//!
//! Everything in this module is dev-only scaffolding to gather *measured evidence* for ADR-009
//! (waveform/spectrogram renderer choice, Wayland input quirks, IPC throughput) before M2 builds
//! production renderers on top of it. It is gated behind the `spike` cargo feature (off by
//! default) so it never ships and never affects the production command surface or `just check`
//! with default features — see `CLAUDE.md` / `tickets/T-007-platform-spike.md`.
//!
//! `src-tauri` stays a thin command layer even here: the synthetic-data generation below has no
//! real business meaning, it exists purely to produce plausible, sized payloads for the
//! measurements the ticket asks for.

pub mod binary;
mod commands;
mod dto;

pub use commands::*;
pub use dto::{SpikeEnv, WaveformMeta};

//! Pure DSP: no I/O, no threads, no allocation inside process().
//!
//! - [`fp`]: the flush-to-zero / denormals-are-zero guard for audio threads and offline renders.
//! - [`resample`]: fixed-ratio streaming sample-rate conversion (document → device rate).
//! - [`capture_resample`]: push-based streaming resampling for the capture-writer thread (H-06,
//!   device rate → document rate) — off the audio thread only, never the RT input callback.
//! - [`eq`]: parametric-EQ biquads, Butterworth cascades and smoothed bands (SPEC-015).
//! - [`loudness`]: BS.1770/EBU R128 integrated/short-term/momentary loudness, LRA, sample and
//!   true peak (S4-01) — off the audio thread only (job/edit-op code, not `process()`).
//! - [`acx`]: ACX / Audible submission pass/fail rules (RMS, sample peak, noise floor; S4-03) —
//!   off the audio thread only, over a whole (source or rack-processed) buffer.
//! - [`dither`]: deterministic TPDF dither/quantizer for 16/24-bit save and export (ADR-001 §4;
//!   H-02 moved this from `vox-io`) — off the audio thread only.

pub mod acx;
pub mod capture_resample;
pub mod dither;
pub mod dynamics;
pub mod eq;
pub mod fp;
pub mod loudness;
pub mod nr;
pub mod resample;
pub mod true_peak;

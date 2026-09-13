//! Pure DSP: no I/O, no threads, no allocation inside process().
//!
//! - [`fp`]: the flush-to-zero / denormals-are-zero guard for audio threads and offline renders.
//! - [`resample`]: fixed-ratio streaming sample-rate conversion (document → device rate).
//! - [`eq`]: parametric-EQ biquads, Butterworth cascades and smoothed bands (SPEC-015).
//! - [`loudness`]: BS.1770/EBU R128 integrated/short-term/momentary loudness, LRA, sample and
//!   true peak (S4-01) — off the audio thread only (job/edit-op code, not `process()`).

pub mod dynamics;
pub mod eq;
pub mod fp;
pub mod loudness;
pub mod nr;
pub mod resample;
pub mod true_peak;

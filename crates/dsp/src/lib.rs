//! Pure DSP: no I/O, no threads, no allocation inside process().
//!
//! - [`fp`]: the flush-to-zero / denormals-are-zero guard for audio threads and offline renders.
//! - [`resample`]: fixed-ratio streaming sample-rate conversion (document → device rate).
//! - [`eq`]: parametric-EQ biquads, Butterworth cascades and smoothed bands (SPEC-015).

pub mod dynamics;
pub mod eq;
pub mod fp;
pub mod nr;
pub mod resample;
pub mod true_peak;

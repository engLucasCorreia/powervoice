//! Pure DSP: no I/O, no threads, no allocation inside process().
//!
//! - [`fp`]: the flush-to-zero / denormals-are-zero guard for audio threads and offline renders.
//! - [`resample`]: fixed-ratio streaming sample-rate conversion (document → device rate).

pub mod fp;
pub mod resample;

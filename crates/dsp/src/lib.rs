//! Pure DSP: no I/O, no threads, no allocation inside process().
//!
//! - [`analyzer`]: live output spectrum analyzer math (T-208, SPEC-007 §4.8): BH4-windowed real
//!   FFT, 1/24-octave band reduction, power-domain Fast/Medium/Slow averaging — off the audio
//!   thread only (the engine's analyzer consumer, never `process()`).
//! - [`fp`]: the flush-to-zero / denormals-are-zero guard for audio threads and offline renders.
//! - [`resample`]: fixed-ratio streaming sample-rate conversion (document → device rate).
//! - [`async_resample`]: adjustable-ratio (drift-corrected) resampling of the monitor ring in the
//!   RT output callback (T-107, ADR-002 §6) — allocation-free after construction.
//! - [`capture_resample`]: push-based streaming resampling for the capture-writer thread (H-06,
//!   device rate → document rate) — off the audio thread only, never the RT input callback.
//! - [`eq`]: parametric-EQ biquads, Butterworth cascades and smoothed bands (SPEC-015).
//! - [`loudness`]: BS.1770/EBU R128 integrated/short-term/momentary loudness, LRA, sample and
//!   true peak (S4-01) — off the audio thread only (job/edit-op code, not `process()`).
//! - [`acx`]: ACX / Audible submission pass/fail rules (RMS, sample peak, noise floor; S4-03) —
//!   off the audio thread only, over a whole (source or rack-processed) buffer.
//! - [`dither`]: deterministic TPDF dither/quantizer for 16/24-bit save and export (ADR-001 §4;
//!   H-02 moved this from `vox-io`) — off the audio thread only.
//! - [`spectro`]: spectrogram STFT frames, overview mean power, u8 quantization and the zoom → hop
//!   rule (T-204, SPEC-007 §4.2–§4.4) — off the audio thread only (engine tile workers).

pub mod acx;
pub mod analyzer;
pub mod async_resample;
pub mod capture_resample;
pub mod dither;
pub mod dynamics;
pub mod eq;
pub mod fp;
pub mod loudness;
pub mod nr;
pub mod resample;
pub mod spectro;
pub mod true_peak;

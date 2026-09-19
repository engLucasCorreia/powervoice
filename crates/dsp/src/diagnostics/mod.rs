//! Analyzer diagnostics (H-42, SPEC-007 §8): the DSP behind the analyzer's voice statistics,
//! the long-term average spectrum (LTAS) job and the Spectrum Inspector. Off the audio thread
//! only — the engine's analyzer publisher (control thread) and job threads call this, never an
//! audio callback.
//!
//! - [`window`]: Hann / Blackman-Harris / flat-top / rectangular analysis windows;
//! - [`spectrum`]: sine-normalized power spectra at 1 024 … 32 768 points and the Welch LTAS;
//! - [`pitch`]: FFT-accelerated YIN fundamental-frequency estimation;
//! - [`f0_profile`]: YIN's frames → the displayed pitch profile, guarded against octave errors;
//! - [`features`]: tone balance, sibilance / de-esser target, mains hum, rumble;
//! - [`voice`]: the live tracker (smoothed values + statistics over the last N s) and the
//!   offline analysis of a whole buffer, both producing one [`voice::VoiceReport`].

pub mod f0_profile;
pub mod features;
pub mod pitch;
pub mod spectrum;
pub mod voice;
pub mod window;

pub use f0_profile::{F0Frame, F0Profile};
pub use features::{Hum, Sibilance, SpectrumView, ToneBalance};
pub use pitch::{Pitch, Yin};
pub use spectrum::{Ltas, PowerSpectrum};
pub use voice::{
    F0Stats, OfflineAnalysis, OfflineConfig, VoiceReport, VoiceTracker, analyze_buffer,
};
pub use window::WindowKind;

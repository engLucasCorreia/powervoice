//! Spectral noise reduction from a captured noise print (SPEC-014).
//!
//! - [`capture_profile`]: the noise print (§4.1) — per-bin mean power density and log-power
//!   spread of an excerpt at a fixed 8192-point periodic-Hann analysis (hop 2048), encoded as
//!   the versioned 32 824-byte blob v1 (§4.2).
//! - [`ProfileView`]: validated, allocation-free view of a blob.
//! - [`derive_lambda`]: the per-bin noise power for any processing FFT size and rate (§4.3).
//! - [`describe_points`]: the print on the analyzer's 1/24-octave bands (§4.10).
//! - [`SpectralNr`]: the streaming STFT reducer (§4.4–§4.7): √Hann analysis/synthesis, 75 %
//!   overlap, latency N, decision-directed Wiener gain, dB-domain frequency and time smoothing,
//!   frame-start parameter binding, exact N-sample delay line without a print.

mod engine;
mod profile;

pub use engine::{BETA, LAMBDA_FLOOR, NrParams, SpectralNr, XI_MIN};
pub use profile::{
    BLOB_LEN_V1, BLOB_MAGIC, BLOB_VERSION, BlobError, CAPTURE_BINS, CAPTURE_FFT_SIZE, CAPTURE_HOP,
    CaptureError, ENBW_BH4_BINS, HEADER_LEN, MAX_CAPTURE_S, MIN_CAPTURE_S, MIN_CAPTURE_SAMPLES_ABS,
    ProfileView, WINDOW_PERIODIC_HANN, capture_profile, derive_lambda, describe_points,
    max_capture_samples, min_capture_samples,
};

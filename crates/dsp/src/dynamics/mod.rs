//! Shared dynamics engine (SPEC-016 §4, normative for SPEC-013): level detectors, static gain
//! computers, ballistics, the gate core, linear parameter ramps and the sidechain high-pass.
//!
//! Every building block is allocation-free after construction and evolves sample by sample, so
//! the output never depends on how a signal is split into blocks (SPEC-016 §4.2).

pub mod ballistics;
pub mod curves;
pub mod detector;
pub mod gate;
pub mod ramp;
pub mod sidechain;

/// Sliding-maximum window of the peak detector (W_pk).
pub const PEAK_WINDOW_MS: f64 = 5.0;
/// Rectangular mean-square window of the RMS detector (W_rms).
pub const RMS_WINDOW_MS: f64 = 20.0;
/// Linear parameter ramps and section/detection crossfades.
pub const RAMP_MS: f64 = 20.0;
/// Detector and composed levels are clamped to at least this.
pub const LEVEL_FLOOR_DBFS: f64 = -150.0;
/// Lowest gain of the dB-domain sections (Expander, Compressor, Limiter).
pub const GAIN_FLOOR_DB: f64 = -120.0;
/// Fixed AutoGate hysteresis.
pub const AUTOGATE_HYSTERESIS_DB: f64 = 3.0;
/// dB smoothers snap onto their target within this distance.
pub const SNAP_DB: f64 = 1e-6;
/// Linear-gain smoothers snap onto their target within this distance.
pub const SNAP_LIN: f64 = 1e-6;
/// `20·log10(√2)`: a sine's RMS level is this far below its peak level.
pub const SINE_PEAK_TO_RMS_DB: f64 = 3.010_299_956_639_812;
/// Maximum look-ahead (= `RMS_WINDOW_MS`).
pub const LOOKAHEAD_MAX_MS: f64 = 20.0;

/// Window or count length: `max(1, round(ms · fs / 1000))` samples (SPEC-016 §4.2).
pub fn ms_to_samples(ms: f64, sample_rate: f64) -> usize {
    let n = (ms * sample_rate / 1000.0).round();
    if n.is_finite() && n >= 1.0 {
        n as usize
    } else {
        1
    }
}

/// Count length that may be 0 (hold): `round(ms · fs / 1000)` samples.
pub fn ms_to_samples_or_zero(ms: f64, sample_rate: f64) -> u32 {
    let n = (ms * sample_rate / 1000.0).round();
    if n.is_finite() && n > 0.0 {
        n.min(f64::from(u32::MAX)) as u32
    } else {
        0
    }
}

/// One-pole coefficient `α = exp(−1 / (τ · fs))` for a time constant `tau_ms` (63.2 % time).
pub fn time_constant_alpha(tau_ms: f64, sample_rate: f64) -> f64 {
    let n = tau_ms / 1000.0 * sample_rate;
    if n > 0.0 { (-1.0 / n).exp() } else { 0.0 }
}

/// Linear magnitude → dBFS, clamped to [`LEVEL_FLOOR_DBFS`] (0 and below → the floor).
pub fn lin_to_dbfs(x: f64) -> f64 {
    if x > 0.0 {
        (20.0 * x.log10()).max(LEVEL_FLOOR_DBFS)
    } else {
        LEVEL_FLOOR_DBFS
    }
}

/// dB → linear gain.
pub fn db_to_lin(db: f64) -> f64 {
    10f64.powf(db / 20.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conversions() {
        assert_eq!(ms_to_samples(5.0, 48_000.0), 240);
        assert_eq!(ms_to_samples(20.0, 44_100.0), 882);
        assert_eq!(ms_to_samples(0.001, 48_000.0), 1);
        assert_eq!(ms_to_samples_or_zero(0.0, 48_000.0), 0);
        assert_eq!(ms_to_samples_or_zero(0.1, 48_000.0), 5);
        assert!((time_constant_alpha(10.0, 48_000.0) - (-1.0f64 / 480.0).exp()).abs() < 1e-15);
        assert!((lin_to_dbfs(0.1) + 20.0).abs() < 1e-12);
        assert!((lin_to_dbfs(0.0) - LEVEL_FLOOR_DBFS).abs() < f64::EPSILON);
        assert!((lin_to_dbfs(1e-9) - LEVEL_FLOOR_DBFS).abs() < f64::EPSILON);
        assert!((db_to_lin(-6.0) - 0.501_187_233_627_272_2).abs() < 1e-12);
        assert!((SINE_PEAK_TO_RMS_DB - 20.0 * 2f64.sqrt().log10()).abs() < 1e-12);
    }
}

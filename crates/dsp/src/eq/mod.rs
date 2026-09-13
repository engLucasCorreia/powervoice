//! Parametric-EQ building blocks (SPEC-015 §4), f64 coefficients and f64 Direct Form I state.
//!
//! - [`coeffs`]: one function per section type (RBJ cookbook peaking / shelves / HPF / LPF,
//!   bilinear first-order sections, Butterworth cascades) and the cookbook's robust magnitude
//!   form. `process()` and the module's `ResponseCurve` both call these, so the graph is exact.
//! - [`band`]: per-sample smoothed bands: 20 ms linear ramps in log2 Hz / dB / ln Q with
//!   per-sample coefficient recompute while ramping, 20 ms on/off and slope crossfades, sections
//!   that always run, and the bit-exact identity rule (SPEC-015 §4.4, §4.6).

pub mod band;
pub mod coeffs;

/// Ramp and crossfade time of every EQ parameter (SPEC-015 §2.3).
pub const RAMP_MS: f64 = 20.0;
/// Band frequencies above this fraction of the sample rate are processed at it (SPEC-015 §2.4).
pub const MAX_FREQ_RATIO: f64 = 0.49;
/// Highest Butterworth order (48 dB/oct).
pub const MAX_ORDER: usize = 8;
/// Sections of the largest cascade (N = 8: four biquads; N = 7: one first-order + three).
pub const MAX_SECTIONS: usize = 4;
/// State values with a smaller magnitude are set to 0 at the end of every block (SPEC-015 §4.7).
pub const DENORMAL_FLUSH: f64 = 1e-30;

/// Ramp length `round(0.020 · fs)` samples.
pub fn ramp_samples(sample_rate: f64) -> u32 {
    let n = (RAMP_MS * sample_rate / 1000.0).round();
    if n.is_finite() && n > 0.0 {
        n.min(f64::from(u32::MAX)) as u32
    } else {
        0
    }
}

/// The processed frequency: `min(freq_hz, 0.49 · fs)`.
pub fn clamp_freq_hz(freq_hz: f64, sample_rate: f64) -> f64 {
    freq_hz.min(MAX_FREQ_RATIO * sample_rate)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ramp_length_and_clamp() {
        assert_eq!(ramp_samples(48_000.0), 960);
        assert_eq!(ramp_samples(44_100.0), 882);
        assert_eq!(ramp_samples(96_000.0), 1920);
        assert_eq!(ramp_samples(0.0), 0);
        assert_eq!(
            clamp_freq_hz(20_000.0, 22_050.0).to_bits(),
            (0.49f64 * 22_050.0).to_bits()
        );
        assert_eq!(
            clamp_freq_hz(1_000.0, 48_000.0).to_bits(),
            1_000f64.to_bits()
        );
    }
}

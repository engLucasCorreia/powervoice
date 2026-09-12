//! Objective measurements: sample peak, RMS, crest factor, DC offset,
//! loudness (BS.1770 / EBU R128 / Tech 3342 via `ebur128`), noise floor and
//! null test.
//!
//! ## Conventions
//! - **Non-finite input**: if any input sample is `NaN` or infinite, a
//!   measurement that depends on it reports `NaN` rather than silently
//!   dropping the bad sample (which is what a naive `f32::max`-based fold
//!   would do) or laundering it into a plausible-looking finite number.
//!   `NaN` serializes as JSON `null` (see `voxedit-cli analyze --json`).
//! - **Exact digital silence** reports `-inf` (`f64::NEG_INFINITY`), not a
//!   floored sentinel value -- see [`crate::units::linear_to_dbfs`]. `-inf`
//!   serializes as JSON `null` and prints as the text `-inf`.
//! - **Insufficient input** (buffer shorter than the measurement window)
//!   reports `None` from an `Option`-typed measurement, distinct from either
//!   of the above.

use ebur128::{EbuR128, Mode};

use crate::error::TestkitError;
use crate::units::linear_to_dbfs;

/// `true` if any sample is `NaN` or infinite.
fn has_non_finite(samples: &[f32]) -> bool {
    samples.iter().any(|x| !x.is_finite())
}

/// Sample peak level in dBFS (`20*log10(max|x|)`); `NaN` if any sample is
/// non-finite.
pub fn peak_dbfs(samples: &[f32]) -> f64 {
    if has_non_finite(samples) {
        return f64::NAN;
    }
    let peak = samples.iter().fold(0.0f32, |acc, &x| acc.max(x.abs()));
    linear_to_dbfs(f64::from(peak))
}

/// RMS level in dB, full-scale referenced (`20*log10(rms)`); `NaN` if any
/// sample is non-finite.
pub fn rms_db(samples: &[f32]) -> f64 {
    if samples.is_empty() {
        return f64::NEG_INFINITY;
    }
    if has_non_finite(samples) {
        return f64::NAN;
    }
    let mean_sq = samples
        .iter()
        .map(|&x| (f64::from(x)) * (f64::from(x)))
        .sum::<f64>()
        / samples.len() as f64;
    linear_to_dbfs(mean_sq.sqrt())
}

/// Crest factor in dB (`peak_dbfs - rms_db`).
pub fn crest_factor_db(samples: &[f32]) -> f64 {
    peak_dbfs(samples) - rms_db(samples)
}

/// DC offset as a linear value (arithmetic mean of the samples).
pub fn dc_offset(samples: &[f32]) -> f64 {
    if samples.is_empty() {
        return 0.0;
    }
    samples.iter().map(|&x| f64::from(x)).sum::<f64>() / samples.len() as f64
}

/// Loudness measurements for one buffer, as reported by `ebur128`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LoudnessReport {
    pub integrated_lufs: f64,
    /// `None` if the input is shorter than the 400 ms momentary window.
    pub max_momentary_lufs: Option<f64>,
    /// `None` if the input is shorter than the 3 s short-term window.
    pub max_short_term_lufs: Option<f64>,
    pub lra_lu: f64,
    pub true_peak_dbtp: f64,
}

const MOMENTARY_WINDOW_S: f64 = 0.4;
const SHORT_TERM_WINDOW_S: f64 = 3.0;

/// Measures loudness of an interleaved buffer with `channels` channels at
/// `sample_rate`, per ITU-R BS.1770 / EBU R128 (integrated, max momentary,
/// max short-term, LRA per Tech 3342) and 4x-oversampled true peak.
///
/// If any sample is non-finite, every field is `NaN` (see the module docs).
pub fn loudness(
    interleaved: &[f32],
    channels: u32,
    sample_rate: u32,
) -> Result<LoudnessReport, TestkitError> {
    if has_non_finite(interleaved) {
        return Ok(LoudnessReport {
            integrated_lufs: f64::NAN,
            max_momentary_lufs: Some(f64::NAN),
            max_short_term_lufs: Some(f64::NAN),
            lra_lu: f64::NAN,
            true_peak_dbtp: f64::NAN,
        });
    }

    let mode = Mode::I | Mode::LRA | Mode::TRUE_PEAK;
    let mut state = EbuR128::new(channels, sample_rate, mode)?;

    let frame_len = channels as usize;
    let total_frames = interleaved.len() / frame_len;
    let hop_frames = (sample_rate as usize / 10).max(1); // 100 ms hop

    let mut max_momentary = f64::NEG_INFINITY;
    let mut max_short_term = f64::NEG_INFINITY;
    let mut offset = 0usize;
    while offset < total_frames {
        let take = hop_frames.min(total_frames - offset);
        let start = offset * frame_len;
        let end = start + take * frame_len;
        state.add_frames_f32(&interleaved[start..end])?;
        offset += take;

        let momentary = state.loudness_momentary()?;
        if momentary.is_finite() {
            max_momentary = max_momentary.max(momentary);
        }
        let short_term = state.loudness_shortterm()?;
        if short_term.is_finite() {
            max_short_term = max_short_term.max(short_term);
        }
    }

    let integrated_lufs = state.loudness_global()?;
    let lra_lu = state.loudness_range()?;
    let mut true_peak_linear = 0.0f64;
    for channel in 0..channels {
        true_peak_linear = true_peak_linear.max(state.true_peak(channel)?);
    }

    let total_duration_s = total_frames as f64 / f64::from(sample_rate);
    let max_momentary_lufs = (total_duration_s >= MOMENTARY_WINDOW_S).then_some(max_momentary);
    let max_short_term_lufs = (total_duration_s >= SHORT_TERM_WINDOW_S).then_some(max_short_term);

    Ok(LoudnessReport {
        integrated_lufs,
        max_momentary_lufs,
        max_short_term_lufs,
        lra_lu,
        true_peak_dbtp: linear_to_dbfs(true_peak_linear),
    })
}

/// Noise floor: the RMS level (dB, unweighted) of the quietest `window_ms`
/// window in the buffer -- the ACX-style measurement.
///
/// `interleaved` has `channels` interleaved channels; each channel's
/// noise floor is measured independently (over its own quietest window,
/// which need not line up with any other channel's) and the *worst*
/// (loudest / least negative) of them is reported, since that is the one
/// that would fail an ACX-style check.
///
/// Returns `None` if the buffer is shorter than `window_ms`, `Some(NaN)` if
/// any sample is non-finite, `Some(-inf)` if every channel is exact digital
/// silence, and `Some(finite dB)` otherwise.
pub fn noise_floor_db(
    interleaved: &[f32],
    channels: u16,
    sample_rate: u32,
    window_ms: f64,
) -> Option<f64> {
    if channels == 0 {
        return None;
    }
    let channels = usize::from(channels);
    let total_frames = interleaved.len() / channels;
    let window_len = ((window_ms / 1000.0) * f64::from(sample_rate)).round() as usize;
    if window_len == 0 || total_frames < window_len {
        return None;
    }
    if has_non_finite(interleaved) {
        return Some(f64::NAN);
    }

    let mut worst_db = f64::NEG_INFINITY;
    let mut channel_buf = vec![0.0f32; total_frames];
    for ch in 0..channels {
        for (frame, sample) in channel_buf.iter_mut().enumerate() {
            *sample = interleaved[frame * channels + ch];
        }
        let db = channel_noise_floor_db(&channel_buf, window_len);
        if db > worst_db {
            worst_db = db;
        }
    }
    Some(worst_db)
}

/// Sliding-window minimum mean-square, in dB, for one (mono) channel.
///
/// Uses a running sum (O(n)) for the per-sample slide, but re-sums the
/// window from scratch every `window_len` samples: an add/subtract-only
/// running sum accumulates floating-point drift over very long buffers
/// (observed: a quiet window ~30 minutes into a loud signal read ~1.3 dB
/// high), and periodic exact re-summation bounds that drift to at most one
/// window's worth of accumulated error while staying O(n) overall (one
/// O(window_len) resum every window_len steps).
fn channel_noise_floor_db(samples: &[f32], window_len: usize) -> f64 {
    let exact_window_sum_sq = |start: usize| -> f64 {
        samples[start..start + window_len]
            .iter()
            .map(|&x| f64::from(x) * f64::from(x))
            .sum()
    };

    let mut sum_sq = exact_window_sum_sq(0);
    let mut min_mean_sq = sum_sq / window_len as f64;
    let mut since_resum = 0usize;

    for i in window_len..samples.len() {
        let incoming = f64::from(samples[i]) * f64::from(samples[i]);
        let outgoing = f64::from(samples[i - window_len]) * f64::from(samples[i - window_len]);
        sum_sq += incoming - outgoing;
        since_resum += 1;

        if since_resum >= window_len {
            sum_sq = exact_window_sum_sq(i + 1 - window_len);
            since_resum = 0;
        }

        let mean_sq = (sum_sq / window_len as f64).max(0.0);
        if mean_sq < min_mean_sq {
            min_mean_sq = mean_sq;
        }
    }
    linear_to_dbfs(min_mean_sq.sqrt())
}

/// The default ACX/testkit noise-floor window (500 ms, unweighted).
pub const DEFAULT_NOISE_FLOOR_WINDOW_MS: f64 = 500.0;

/// Null test: the maximum absolute sample difference between two
/// equal-length buffers, in dB -- useful for verifying bit-exact / near-exact
/// processing. `NaN` if either buffer contains a non-finite sample.
pub fn null_test_db(a: &[f32], b: &[f32]) -> f64 {
    if has_non_finite(a) || has_non_finite(b) {
        return f64::NAN;
    }
    let max_diff = a
        .iter()
        .zip(b.iter())
        .map(|(&x, &y)| (x - y).abs())
        .fold(0.0f32, f32::max);
    linear_to_dbfs(f64::from(max_diff))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::signal;

    #[test]
    fn mono_sine_peak_and_rms() {
        let s = signal::sine(1000.0, -20.0, 20.0, 48_000).unwrap();
        assert!((peak_dbfs(&s) - (-20.0)).abs() < 0.01);
        assert!((rms_db(&s) - (-23.0103)).abs() < 0.01);
    }

    #[test]
    fn silence_has_zero_dc_and_low_peak() {
        let s = signal::silence(1.0, 48_000).unwrap();
        assert!(dc_offset(&s).abs() < 1e-12);
        assert!(peak_dbfs(&s) < -100.0);
    }

    #[test]
    fn null_test_zero_for_identical_buffers() {
        let s = signal::sine(1000.0, -10.0, 1.0, 48_000).unwrap();
        assert!(null_test_db(&s, &s) < -100.0);
    }

    #[test]
    fn null_test_reports_known_difference() {
        let a = vec![0.0f32; 10];
        let mut b = vec![0.0f32; 10];
        b[3] = 0.5;
        // 20*log10(0.5) = -6.02 dB
        assert!((null_test_db(&a, &b) - (-6.0206)).abs() < 0.01);
    }

    #[test]
    fn nan_sample_does_not_masquerade_as_silence() {
        // Regression test: a NaN used to be silently dropped by an
        // `f32::max`-based fold (peak/null test) or laundered into a
        // plausible-looking -240 dBFS by an over-eager floor in
        // `linear_to_dbfs` (rms/noise floor). All four measurements must
        // now surface it as NaN instead.
        let mut s = signal::sine(1000.0, -20.0, 0.1, 48_000).unwrap();
        s[10] = f32::NAN;

        assert!(peak_dbfs(&s).is_nan());
        assert!(rms_db(&s).is_nan());
        assert!(null_test_db(&s, &s).is_nan());
        assert!(noise_floor_db(&s, 1, 48_000, 50.0).unwrap().is_nan());
    }

    #[test]
    fn infinite_sample_is_also_non_finite() {
        let mut s = signal::sine(1000.0, -20.0, 0.1, 48_000).unwrap();
        s[0] = f32::INFINITY;
        assert!(peak_dbfs(&s).is_nan());
        assert!(rms_db(&s).is_nan());
    }

    #[test]
    fn loudness_reports_nan_for_non_finite_input() {
        let mut s = signal::sine(1000.0, -20.0, 1.0, 48_000).unwrap();
        s[0] = f32::NAN;
        let report = loudness(&s, 1, 48_000).unwrap();
        assert!(report.integrated_lufs.is_nan());
        assert!(report.max_momentary_lufs.unwrap().is_nan());
        assert!(report.true_peak_dbtp.is_nan());
    }

    #[test]
    fn loudness_max_momentary_and_short_term_none_when_too_short() {
        // 200 ms: shorter than the 400 ms momentary window.
        let s = signal::sine(1000.0, -20.0, 0.2, 48_000).unwrap();
        let report = loudness(&s, 1, 48_000).unwrap();
        assert!(report.max_momentary_lufs.is_none());
        assert!(report.max_short_term_lufs.is_none());

        // 1 s: long enough for momentary, still short of the 3 s short-term
        // window.
        let s = signal::sine(1000.0, -20.0, 1.0, 48_000).unwrap();
        let report = loudness(&s, 1, 48_000).unwrap();
        assert!(report.max_momentary_lufs.is_some());
        assert!(report.max_short_term_lufs.is_none());
    }

    #[test]
    fn noise_floor_none_when_shorter_than_window() {
        let s = signal::silence(0.1, 48_000).unwrap(); // 100 ms < 500 ms default window
        assert!(noise_floor_db(&s, 1, 48_000, DEFAULT_NOISE_FLOOR_WINDOW_MS).is_none());
    }

    #[test]
    fn noise_floor_reports_worst_channel() {
        // Left channel: quiet white noise. Right channel: louder white
        // noise. The interleaved stereo noise floor must report the
        // *louder* (right) channel, not an average or the left channel.
        let quiet = signal::white_noise(1, -70.0, 2.0, 48_000).unwrap();
        let loud = signal::white_noise(2, -40.0, 2.0, 48_000).unwrap();
        let mut interleaved = Vec::with_capacity(quiet.len() * 2);
        for (l, r) in quiet.iter().zip(loud.iter()) {
            interleaved.push(*l);
            interleaved.push(*r);
        }

        let floor = noise_floor_db(&interleaved, 2, 48_000, DEFAULT_NOISE_FLOOR_WINDOW_MS).unwrap();
        assert!(
            (floor - (-40.0)).abs() <= 1.0,
            "expected worst-channel floor near -40 dB, got {floor:.4}"
        );
    }

    #[test]
    #[allow(clippy::float_cmp)] // deliberate exact-value check, not a tolerance comparison
    fn noise_floor_sliding_window_does_not_drift_over_a_long_buffer() {
        // Regression test: an add/subtract-only running sum accumulates
        // floating-point drift proportional to how long it has been running
        // (verified empirically: a naive running sum over 1M samples of
        // -3 dBFS white noise followed by exact digital silence reads
        // roughly -134 dB instead of the true -inf). Periodic exact
        // re-summation must reach exactly -inf once a full window of true
        // silence has been seen, regardless of what came before.
        let sample_rate = 48_000;
        let mut s = signal::white_noise(1, -3.0, 1_000_000.0 / f64::from(sample_rate), sample_rate)
            .unwrap();
        s.extend(vec![0.0f32; sample_rate as usize]); // 1s of exact silence

        let floor = noise_floor_db(&s, 1, sample_rate, DEFAULT_NOISE_FLOOR_WINDOW_MS).unwrap();
        assert_eq!(
            floor,
            f64::NEG_INFINITY,
            "noise floor {floor:.4} dB, expected exactly -inf (drift check)"
        );
    }
}

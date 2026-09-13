//! ACX / Audible submission pass/fail rules (S4-03; docs/references.md ACX section): RMS in
//! `[-23, -18]` dB, sample peak `<= -3` dBFS, noise floor (quietest 500 ms window, unweighted)
//! `<= -60` dB. Mono only (PROMPT §2: single-file mono editor) over a whole buffer the caller has
//! already picked — the raw document, or the rack-processed signal (S4-01's "processed by
//! default, with a source toggle" convention; `src-tauri::loudness::LoudnessService` renders
//! whichever one is asked for and hands the result here).
//!
//! Conventions match `loudness` and `vox-testkit::measure` exactly: `dB = 20*log10(rms)`; exact
//! digital silence is `-inf`; any non-finite sample makes every measurement `NaN` (never silently
//! dropped, never laundered into a plausible-looking finite number); a scope shorter than the
//! noise-floor window reports [`AcxRuleStatus::TooShort`] with no measurement at all, distinct
//! from either of the above.
//!
//! Thresholds are inclusive (`RMS == -18.0` passes; `-17.9` does not) — the ticket's own boundary
//! acceptance criteria pins this down (docs/references.md doesn't spell out open vs. closed
//! bounds).

use crate::loudness::linear_to_dbfs;

/// ACX RMS floor, dB (`20*log10(rms)`), inclusive.
pub const ACX_RMS_MIN_DBFS: f64 = -23.0;
/// ACX RMS ceiling, dB, inclusive.
pub const ACX_RMS_MAX_DBFS: f64 = -18.0;
/// ACX sample-peak ceiling, dBFS, inclusive.
pub const ACX_PEAK_MAX_DBFS: f64 = -3.0;
/// ACX noise-floor ceiling, dB, inclusive.
pub const ACX_NOISE_FLOOR_MAX_DB: f64 = -60.0;
/// The noise-floor measurement window (MEMORY.md: "we use 500 ms, unweighted").
pub const ACX_NOISE_FLOOR_WINDOW_MS: f64 = 500.0;

/// Absorbs floating-point noise at an exact boundary (e.g. a `white_noise`-generated -18.0 dBFS
/// buffer measuring back as -17.999999999305434, not exactly -18.0) without weakening the rule:
/// many orders of magnitude below the ticket's own ±0.1 dB *measurement* tolerance, so a signal
/// that's actually 0.1 dB over a limit still fails.
const BOUNDARY_EPSILON_DB: f64 = 1e-6;

/// One rule's outcome. RMS is the only rule with a floor as well as a ceiling — [`Self::TooLow`]
/// never occurs for peak or noise floor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AcxRuleStatus {
    Pass,
    TooLow,
    TooHigh,
    /// A non-finite sample was in scope (the measurement is `NaN`) — always a fail, never a
    /// silent pass (a naive comparison against `NaN` is false either way).
    Invalid,
    /// The scope is shorter than the measurement window (noise floor only; no measurement at
    /// all, distinct from a `NaN`/`-inf` one).
    TooShort,
}

/// One rule's row: the measured value (`None` only for [`AcxRuleStatus::TooShort`]) and its
/// [`AcxRuleStatus`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AcxRuleReport {
    pub measured_db: Option<f64>,
    pub status: AcxRuleStatus,
}

impl AcxRuleReport {
    fn pass(measured_db: f64) -> Self {
        Self {
            measured_db: Some(measured_db),
            status: AcxRuleStatus::Pass,
        }
    }

    fn is_pass(&self) -> bool {
        self.status == AcxRuleStatus::Pass
    }
}

/// The full ACX check: one row per rule, over one buffer.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AcxReport {
    pub rms: AcxRuleReport,
    pub peak: AcxRuleReport,
    pub noise_floor: AcxRuleReport,
}

impl AcxReport {
    /// Whole-document pass/fail: every rule must pass.
    pub fn passes(&self) -> bool {
        self.rms.is_pass() && self.peak.is_pass() && self.noise_floor.is_pass()
    }
}

/// Evaluates a mono buffer against the ACX rules. `sample_rate_hz` is only used for the
/// noise-floor window length.
pub fn evaluate(samples: &[f32], sample_rate_hz: u32) -> AcxReport {
    let rms = rms_rule(rms_dbfs(samples));
    let peak = upper_bound_rule(peak_dbfs(samples), ACX_PEAK_MAX_DBFS);
    let noise_floor = match noise_floor_db(samples, sample_rate_hz, ACX_NOISE_FLOOR_WINDOW_MS) {
        None => AcxRuleReport {
            measured_db: None,
            status: AcxRuleStatus::TooShort,
        },
        Some(v) => upper_bound_rule(v, ACX_NOISE_FLOOR_MAX_DB),
    };
    AcxReport {
        rms,
        peak,
        noise_floor,
    }
}

fn rms_rule(measured_db: f64) -> AcxRuleReport {
    let status = if measured_db.is_nan() {
        AcxRuleStatus::Invalid
    } else if measured_db < ACX_RMS_MIN_DBFS - BOUNDARY_EPSILON_DB {
        AcxRuleStatus::TooLow
    } else if measured_db > ACX_RMS_MAX_DBFS + BOUNDARY_EPSILON_DB {
        AcxRuleStatus::TooHigh
    } else {
        AcxRuleStatus::Pass
    };
    if status == AcxRuleStatus::Pass {
        return AcxRuleReport::pass(measured_db);
    }
    AcxRuleReport {
        measured_db: Some(measured_db),
        status,
    }
}

/// Shared by peak and noise floor: both rules only ever fail high.
fn upper_bound_rule(measured_db: f64, max_db: f64) -> AcxRuleReport {
    let status = if measured_db.is_nan() {
        AcxRuleStatus::Invalid
    } else if measured_db > max_db + BOUNDARY_EPSILON_DB {
        AcxRuleStatus::TooHigh
    } else {
        AcxRuleStatus::Pass
    };
    AcxRuleReport {
        measured_db: Some(measured_db),
        status,
    }
}

fn has_non_finite(samples: &[f32]) -> bool {
    samples.iter().any(|x| !x.is_finite())
}

/// RMS level, dB, full-scale referenced (`20*log10(rms)`); `-inf` for an empty buffer, `NaN` if
/// any sample is non-finite — same convention as `vox-testkit::measure::rms_db`.
fn rms_dbfs(samples: &[f32]) -> f64 {
    if samples.is_empty() {
        return f64::NEG_INFINITY;
    }
    if has_non_finite(samples) {
        return f64::NAN;
    }
    let mean_sq = samples
        .iter()
        .map(|&x| f64::from(x) * f64::from(x))
        .sum::<f64>()
        / samples.len() as f64;
    linear_to_dbfs(mean_sq.sqrt())
}

/// Sample peak level, dBFS (`20*log10(max|x|)`); `NaN` if any sample is non-finite — same
/// convention as `vox-testkit::measure::peak_dbfs`.
fn peak_dbfs(samples: &[f32]) -> f64 {
    if has_non_finite(samples) {
        return f64::NAN;
    }
    let peak = samples.iter().fold(0.0f32, |acc, &x| acc.max(x.abs()));
    linear_to_dbfs(f64::from(peak))
}

/// Noise floor: the RMS level (dB, unweighted) of the quietest `window_ms` window in the buffer.
/// `None` if the buffer is shorter than the window; `Some(NaN)` if any sample is non-finite;
/// `Some(-inf)` for exact digital silence; `Some(finite dB)` otherwise. Mono only — matches
/// `vox-testkit::measure::noise_floor_db`'s per-channel algorithm with `channels == 1` (no
/// worst-of-channels reduction needed).
fn noise_floor_db(samples: &[f32], sample_rate_hz: u32, window_ms: f64) -> Option<f64> {
    let window_len = ((window_ms / 1000.0) * f64::from(sample_rate_hz)).round() as usize;
    if window_len == 0 || samples.len() < window_len {
        return None;
    }
    if has_non_finite(samples) {
        return Some(f64::NAN);
    }
    Some(sliding_min_rms_db(samples, window_len))
}

/// Sliding-window minimum mean-square, in dB. Mirrors
/// `vox-testkit::measure::channel_noise_floor_db`'s periodic-exact-resum approach (an
/// add/subtract-only running sum drifts over long buffers; re-summing from scratch every
/// `window_len` samples bounds that drift to at most one window's worth of error while staying
/// O(n) overall).
fn sliding_min_rms_db(samples: &[f32], window_len: usize) -> f64 {
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

#[cfg(test)]
mod tests {
    use super::*;
    use vox_testkit::signal;

    const RATE: u32 = 48_000;

    /// A signal whose whole-buffer RMS is exactly `target_rms_dbfs` (white noise: exact target
    /// RMS by construction, per `vox-testkit::signal::white_noise`'s own docs).
    fn rms_signal(target_rms_dbfs: f64, duration_s: f64) -> Vec<f32> {
        signal::white_noise(1, target_rms_dbfs, duration_s, RATE).unwrap()
    }

    /// Ticket AC boundary: "RMS -18.0/-17.9" -> pass/fail. `white_noise`'s RMS is exact by
    /// construction, so the measured value should agree with the target to a tiny fraction of the
    /// ±0.1 dB tolerance the ticket allows for the *measurement*.
    #[test]
    fn rms_boundary_minus_18_passes_minus_17_9_fails() {
        let pass = evaluate(&rms_signal(-18.0, 2.0), RATE);
        assert_eq!(pass.rms.status, AcxRuleStatus::Pass);
        assert!((pass.rms.measured_db.unwrap() - (-18.0)).abs() <= 0.1);

        let fail = evaluate(&rms_signal(-17.9, 2.0), RATE);
        assert_eq!(fail.rms.status, AcxRuleStatus::TooHigh);
        assert!((fail.rms.measured_db.unwrap() - (-17.9)).abs() <= 0.1);
    }

    /// Symmetric coverage of the RMS floor (not itself a ticket boundary case, but the same rule
    /// has two sides).
    #[test]
    fn rms_boundary_minus_23_passes_minus_23_1_fails() {
        let pass = evaluate(&rms_signal(-23.0, 2.0), RATE);
        assert_eq!(pass.rms.status, AcxRuleStatus::Pass);

        let fail = evaluate(&rms_signal(-23.1, 2.0), RATE);
        assert_eq!(fail.rms.status, AcxRuleStatus::TooLow);
    }

    /// Ticket AC boundary: "peak -3.0/-2.9" -> pass/fail. `sine`'s peak is exact by construction.
    #[test]
    fn peak_boundary_minus_3_passes_minus_2_9_fails() {
        let pass = evaluate(&signal::sine(1000.0, -3.0, 1.0, RATE).unwrap(), RATE);
        assert_eq!(pass.peak.status, AcxRuleStatus::Pass);
        assert!((pass.peak.measured_db.unwrap() - (-3.0)).abs() <= 0.1);

        let fail = evaluate(&signal::sine(1000.0, -2.9, 1.0, RATE).unwrap(), RATE);
        assert_eq!(fail.peak.status, AcxRuleStatus::TooHigh);
        assert!((fail.peak.measured_db.unwrap() - (-2.9)).abs() <= 0.1);
    }

    /// Builds a buffer whose quietest 500 ms window is an exact analytic sine at
    /// `target_rms_dbfs` RMS (`peak = rms + 3.0103 dB`), bracketed by louder sine so the sliding
    /// window can never straddle the boundary and read lower than the embedded tone itself.
    fn noise_floor_signal(target_rms_dbfs: f64) -> Vec<f32> {
        const SINE_PEAK_TO_RMS_DB: f64 = 3.0103;
        let loud = signal::sine(400.0, -6.0, 1.0, RATE).unwrap(); // RMS ~ -9 dB, well above target
        let quiet = signal::sine(1000.0, target_rms_dbfs + SINE_PEAK_TO_RMS_DB, 0.5, RATE).unwrap();
        assert_eq!(quiet.len(), (0.5 * f64::from(RATE)) as usize); // exactly one window length
        let mut out = loud.clone();
        out.extend_from_slice(&quiet);
        out.extend_from_slice(&loud);
        out
    }

    /// Ticket AC boundary: "noise floor -60.0/-59.9" -> pass/fail.
    #[test]
    fn noise_floor_boundary_minus_60_passes_minus_59_9_fails() {
        let pass = evaluate(&noise_floor_signal(-60.0), RATE);
        assert_eq!(pass.noise_floor.status, AcxRuleStatus::Pass);
        assert!((pass.noise_floor.measured_db.unwrap() - (-60.0)).abs() <= 0.1);

        let fail = evaluate(&noise_floor_signal(-59.9), RATE);
        assert_eq!(fail.noise_floor.status, AcxRuleStatus::TooHigh);
        assert!((fail.noise_floor.measured_db.unwrap() - (-59.9)).abs() <= 0.1);
    }

    #[test]
    fn noise_floor_too_short_reports_indeterminate_not_a_measurement() {
        let s = signal::silence(0.1, RATE).unwrap(); // 100 ms < 500 ms window
        let report = evaluate(&s, RATE);
        assert_eq!(report.noise_floor.status, AcxRuleStatus::TooShort);
        assert_eq!(report.noise_floor.measured_db, None);
        assert!(!report.passes());
    }

    #[test]
    fn a_file_passing_every_rule_passes_overall() {
        // 4 s of -20 dB RMS white noise (mid-band; a uniformly loud signal's own noise floor is
        // just its RMS, nowhere near -60 dB) plus one exact-silence 500 ms window (a room-tone
        // style gap) — small enough a fraction of the whole file that it barely moves the
        // whole-file RMS, but it is the quietest window, and it passes the noise-floor rule
        // trivially (exact silence).
        let mut s = rms_signal(-20.0, 4.0);
        s.extend(signal::silence(0.5, RATE).unwrap());
        let report = evaluate(&s, RATE);
        assert_eq!(report.rms.status, AcxRuleStatus::Pass);
        assert_eq!(report.peak.status, AcxRuleStatus::Pass);
        assert_eq!(report.noise_floor.status, AcxRuleStatus::Pass);
        assert_eq!(report.noise_floor.measured_db, Some(f64::NEG_INFINITY));
        assert!(report.passes());
    }

    #[test]
    fn non_finite_sample_fails_every_rule_as_invalid_not_a_silent_pass() {
        let mut s = signal::sine(1000.0, -20.0, 1.0, RATE).unwrap();
        s[10] = f32::NAN;
        let report = evaluate(&s, RATE);
        assert_eq!(report.rms.status, AcxRuleStatus::Invalid);
        assert_eq!(report.peak.status, AcxRuleStatus::Invalid);
        assert_eq!(report.noise_floor.status, AcxRuleStatus::Invalid);
        assert!(!report.passes());
    }

    #[test]
    fn digital_silence_fails_rms_too_low_but_passes_peak_and_noise_floor() {
        let s = signal::silence(1.0, RATE).unwrap();
        let report = evaluate(&s, RATE);
        assert_eq!(report.rms.status, AcxRuleStatus::TooLow);
        assert_eq!(report.rms.measured_db, Some(f64::NEG_INFINITY));
        assert_eq!(report.peak.status, AcxRuleStatus::Pass);
        assert_eq!(report.noise_floor.status, AcxRuleStatus::Pass);
        assert!(!report.passes());
    }
}

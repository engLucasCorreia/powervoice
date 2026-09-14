//! Loopback latency calibration (SPEC-022 §2.14, §4.7): the exponential sine sweep the engine
//! plays after the rack, and the band-limited GCC-PHAT analysis that measures the residual
//! recording offset from what came back.
//!
//! Off the audio thread only: [`sweep`] runs when an output stream is built (control thread), and
//! [`analyze`] runs on a job worker once the capture is complete (it allocates FFT buffers).

use realfft::RealFftPlanner;
use realfft::num_complex::Complex;

/// §3 `calib_sweep`: start frequency.
pub const SWEEP_F0_HZ: f64 = 100.0;
/// §3 `calib_sweep`: end frequency cap (the actual end is `min(12 kHz, 0.45 · rate)`).
pub const SWEEP_F1_MAX_HZ: f64 = 12_000.0;
/// §3 `calib_sweep`: duration.
pub const SWEEP_SECONDS: f64 = 1.0;
/// §3 `calib_sweep`: peak level (−12 dBFS).
pub const SWEEP_PEAK_DBFS: f64 = -12.0;
/// §3 `calib_sweep`: raised-cosine fades at both ends.
pub const SWEEP_FADE_MS: f64 = 10.0;
/// §3 `calib_reps`.
pub const REPS: usize = 5;
/// §3 `calib_spacing_s`: repetition start spacing.
pub const SPACING_SECONDS: f64 = 1.6;
/// §3 `calib_lag_window_ms`: earliest lag searched.
pub const LAG_MIN_MS: f64 = -50.0;
/// §3 `calib_lag_window_ms`: latest lag searched.
pub const LAG_MAX_MS: f64 = 500.0;
/// §3 `calib_psr_min`: a repetition is valid at or above this peak-to-sidelobe ratio.
pub const PSR_MIN: f64 = 2.0;
/// §3 `calib_psr_min`: sidelobes are searched outside ±1 ms of the peak.
pub const SIDELOBE_EXCLUSION_MS: f64 = 1.0;
/// §3 `calib_agree_samples`: agreement tolerance around the median.
pub const AGREE_SAMPLES: f64 = 2.0;
/// §3 `calib_min_confidence`: acceptance threshold.
pub const MIN_CONFIDENCE: f64 = 0.8;
/// §3 `calib_weak_psr`: the "signal was weak" warning below this median PSR.
pub const WEAK_PSR: f64 = 3.0;
/// SPEC-002 §3 `clip_threshold` (the clipping warning).
pub const CLIP_THRESHOLD: f32 = 0.999_90;

/// End frequency of the sweep at `rate_hz`.
pub fn sweep_f1_hz(rate_hz: u32) -> f64 {
    SWEEP_F1_MAX_HZ.min(0.45 * f64::from(rate_hz))
}

/// The calibration sweep at `rate_hz` (§4.7): `a · sin(2π f₀ K (e^{t/K} − 1))`,
/// `K = T / ln(f₁/f₀)`, with 10 ms raised-cosine fades at both ends.
pub fn sweep(rate_hz: u32) -> Vec<f32> {
    let rate = f64::from(rate_hz.max(1));
    let n = (SWEEP_SECONDS * rate).round() as usize;
    let f1 = sweep_f1_hz(rate_hz);
    let k = SWEEP_SECONDS / (f1 / SWEEP_F0_HZ).ln();
    let amp = 10f64.powf(SWEEP_PEAK_DBFS / 20.0);
    let fade = ((SWEEP_FADE_MS / 1000.0) * rate).round().max(1.0) as usize;
    (0..n)
        .map(|i| {
            let t = i as f64 / rate;
            let phase = std::f64::consts::TAU * SWEEP_F0_HZ * k * ((t / k).exp() - 1.0);
            let from_end = n - 1 - i;
            let w = if i < fade {
                0.5 * (1.0 - (std::f64::consts::PI * i as f64 / fade as f64).cos())
            } else if from_end < fade {
                0.5 * (1.0 - (std::f64::consts::PI * from_end as f64 / fade as f64).cos())
            } else {
                1.0
            };
            (amp * w * phase.sin()) as f32
        })
        .collect()
}

/// Output frames between repetition starts at `rate_hz`.
pub fn spacing_frames(rate_hz: u32) -> u64 {
    (SPACING_SECONDS * f64::from(rate_hz)).round() as u64
}

/// Why a run was rejected (§2.14 step 3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CalibrationReject {
    /// Every repetition was invalid and the input carried no signal at all (digital silence).
    NoSignal,
    /// Too few repetitions agreed (confidence below [`MIN_CONFIDENCE`]).
    LowConfidence,
}

/// One repetition's measurement.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RepMeasurement {
    /// Measured lag in samples (sub-sample, parabolic interpolation).
    pub lag_samples: f64,
    /// Peak-to-sidelobe ratio.
    pub psr: f64,
    /// `psr ≥ PSR_MIN`.
    pub valid: bool,
}

/// The analysis result (§4.9 `calibration_result`).
#[derive(Clone, Debug, PartialEq)]
pub struct CalibrationResult {
    /// Median lag of the valid repetitions in samples (0 when none is valid).
    pub offset_samples: f64,
    /// `offset_samples / rate · 1000`, unrounded.
    pub offset_ms: f64,
    /// Repetitions within ±2 samples of the median, over all repetitions.
    pub confidence: f64,
    /// Repetitions agreeing with the median.
    pub reps_agreeing: u32,
    /// Median PSR over the valid repetitions (0 when none).
    pub psr_median: f64,
    /// Highest recorded level in the analysis windows (dBFS, `-inf` for silence).
    pub peak_dbfs: f64,
    /// Some input sample reached the clip threshold (warning).
    pub clipped: bool,
    /// Median PSR below [`WEAK_PSR`] on an accepted run (warning).
    pub weak: bool,
    /// Confidence ≥ [`MIN_CONFIDENCE`].
    pub accepted: bool,
    /// Why it was rejected (`None` when accepted).
    pub reject: Option<CalibrationReject>,
    /// Per-repetition detail.
    pub reps: Vec<RepMeasurement>,
}

/// Measures the residual offset (§4.7). `recording` is the input captured with automatic
/// alignment; `rep_starts[i]` is the recording index aligned to the heard time of repetition `i`
/// (it may lie outside the recording — missing samples count as silence). `sweep` is the played
/// signal (same rate as the recording).
pub fn analyze(
    recording: &[f32],
    rep_starts: &[i64],
    sweep: &[f32],
    rate_hz: u32,
) -> CalibrationResult {
    let rate = f64::from(rate_hz.max(1));
    let n = sweep.len();
    let tau_min = (LAG_MIN_MS / 1000.0 * rate).round() as i64;
    let tau_max = (LAG_MAX_MS / 1000.0 * rate).round() as i64;
    let span = (tau_max - tau_min) as usize;
    let window_len = n + span;
    let fft_len = (window_len + n).next_power_of_two();
    let mut planner = RealFftPlanner::<f64>::new();
    let forward = planner.plan_fft_forward(fft_len);
    let inverse = planner.plan_fft_inverse(fft_len);
    let bins = fft_len / 2 + 1;

    let mut sweep_buf = vec![0.0f64; fft_len];
    for (d, &s) in sweep_buf.iter_mut().zip(sweep) {
        *d = f64::from(s);
    }
    let mut sweep_spec = vec![Complex::new(0.0, 0.0); bins];
    // A planner-made plan only fails on mismatched buffer lengths, which these never are.
    let _ = forward.process(&mut sweep_buf, &mut sweep_spec);

    let f0 = SWEEP_F0_HZ;
    let f1 = sweep_f1_hz(rate_hz);
    let bin_hz = rate / fft_len as f64;
    let exclusion = (SIDELOBE_EXCLUSION_MS / 1000.0 * rate).round() as usize;

    let mut buf = vec![0.0f64; fft_len];
    let mut spec = vec![Complex::new(0.0, 0.0); bins];
    let mut corr = vec![0.0f64; fft_len];
    let mut peak = 0.0f32;
    let mut clipped = false;
    let mut any_signal = false;
    let mut reps = Vec::with_capacity(rep_starts.len());
    for &start in rep_starts {
        buf.fill(0.0);
        let first = start + tau_min;
        for (j, slot) in buf.iter_mut().take(window_len).enumerate() {
            let idx = first + j as i64;
            if idx >= 0
                && let Some(&x) = recording.get(idx as usize)
            {
                *slot = f64::from(x);
                let a = x.abs();
                peak = peak.max(a);
                clipped |= a >= CLIP_THRESHOLD;
                any_signal |= x != 0.0;
            }
        }
        let _ = forward.process(&mut buf, &mut spec);
        for (k, (r, s)) in spec.iter_mut().zip(&sweep_spec).enumerate() {
            let f = k as f64 * bin_hz;
            let cross = *r * s.conj();
            let mag = cross.norm();
            *r = if f >= f0 && f <= f1 && mag > 1e-20 {
                cross / mag
            } else {
                Complex::new(0.0, 0.0)
            };
        }
        // realfft's inverse wants a purely real DC and Nyquist bin.
        spec[0].im = 0.0;
        if let Some(last) = spec.last_mut() {
            last.im = 0.0;
        }
        let _ = inverse.process(&mut spec, &mut corr);
        // Valid (non-wrapping) offsets of the sweep inside the window: m ∈ [0, span].
        let valid = &corr[..=span];
        let (m, peak_val) = valid
            .iter()
            .enumerate()
            .fold((0usize, 0.0f64), |(bm, bv), (i, &v)| {
                if v.abs() > bv { (i, v.abs()) } else { (bm, bv) }
            });
        let sidelobe = valid
            .iter()
            .enumerate()
            .filter(|(i, _)| i.abs_diff(m) > exclusion)
            .fold(0.0f64, |acc, (_, &v)| acc.max(v.abs()));
        let psr = if sidelobe > 0.0 {
            peak_val / sidelobe
        } else if peak_val > 0.0 {
            f64::INFINITY
        } else {
            0.0
        };
        let frac = if m > 0 && m < span {
            let (a, b, c) = (valid[m - 1].abs(), valid[m].abs(), valid[m + 1].abs());
            let den = a - 2.0 * b + c;
            if den.abs() > f64::EPSILON {
                (0.5 * (a - c) / den).clamp(-0.5, 0.5)
            } else {
                0.0
            }
        } else {
            0.0
        };
        let lag = (tau_min + m as i64) as f64 + frac;
        reps.push(RepMeasurement {
            lag_samples: lag,
            psr,
            valid: peak_val > 0.0 && psr >= PSR_MIN,
        });
    }

    let mut lags: Vec<f64> = reps
        .iter()
        .filter(|r| r.valid)
        .map(|r| r.lag_samples)
        .collect();
    let mut psrs: Vec<f64> = reps.iter().filter(|r| r.valid).map(|r| r.psr).collect();
    let offset_samples = median(&mut lags).unwrap_or(0.0);
    let psr_median = median(&mut psrs).unwrap_or(0.0);
    let total = rep_starts.len().max(1) as f64;
    let agreeing = reps
        .iter()
        .filter(|r| r.valid && (r.lag_samples - offset_samples).abs() <= AGREE_SAMPLES)
        .count() as u32;
    let confidence = f64::from(agreeing) / total;
    let accepted = confidence >= MIN_CONFIDENCE;
    let reject = if accepted {
        None
    } else if !any_signal {
        Some(CalibrationReject::NoSignal)
    } else {
        Some(CalibrationReject::LowConfidence)
    };
    CalibrationResult {
        offset_samples,
        offset_ms: offset_samples / rate * 1000.0,
        confidence,
        reps_agreeing: agreeing,
        psr_median,
        peak_dbfs: if peak > 0.0 {
            20.0 * f64::from(peak).log10()
        } else {
            f64::NEG_INFINITY
        },
        clipped,
        weak: accepted && psr_median < WEAK_PSR,
        accepted,
        reject,
        reps,
    }
}

fn median(values: &mut [f64]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    values.sort_by(f64::total_cmp);
    let mid = values.len() / 2;
    Some(if values.len() % 2 == 1 {
        values[mid]
    } else {
        0.5 * (values[mid - 1] + values[mid])
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use vox_testkit::prng::Pcg32;

    /// One synthetic run: 5 repetitions starting at `base + i · spacing` in "heard" time, looped
    /// back with `gain_db` and delayed by `delay` samples, plus uniform white noise at
    /// `noise_rms_dbfs`. `mute[i]` silences repetition `i`; `path` post-processes the looped
    /// signal (filters, echoes) before the noise is added.
    struct Run {
        rate: u32,
        delay: i64,
        gain_db: f64,
        noise_rms_dbfs: Option<f64>,
        seed: u64,
        mute: [bool; REPS],
        path: fn(&[f64], u32) -> Vec<f64>,
    }

    fn identity(x: &[f64], _: u32) -> Vec<f64> {
        x.to_vec()
    }

    impl Run {
        fn new(rate: u32, delay: i64) -> Self {
            Self {
                rate,
                delay,
                gain_db: -20.0,
                noise_rms_dbfs: Some(-40.0),
                seed: 1,
                mute: [false; REPS],
                path: identity,
            }
        }

        fn analyze(&self) -> CalibrationResult {
            let s = sweep(self.rate);
            let base = (0.2 * f64::from(self.rate)) as i64;
            let spacing = spacing_frames(self.rate) as i64;
            let len = (base + spacing * REPS as i64 + f64::from(self.rate) as i64) as usize;
            let mut looped = vec![0.0f64; len];
            let g = 10f64.powf(self.gain_db / 20.0);
            let starts: Vec<i64> = (0..REPS as i64).map(|i| base + i * spacing).collect();
            for (i, &st) in starts.iter().enumerate() {
                if self.mute[i] {
                    continue;
                }
                for (j, &x) in s.iter().enumerate() {
                    let idx = st + self.delay + j as i64;
                    if idx >= 0 && (idx as usize) < len {
                        looped[idx as usize] += g * f64::from(x);
                    }
                }
            }
            let mut rec: Vec<f64> = (self.path)(&looped, self.rate);
            if let Some(db) = self.noise_rms_dbfs {
                // Uniform in [-a, a) has RMS a/√3.
                let a = 10f64.powf(db / 20.0) * 3f64.sqrt();
                let mut rng = Pcg32::new(self.seed, 7);
                for x in &mut rec {
                    *x += a * rng.next_signed();
                }
            }
            let rec: Vec<f32> = rec.iter().map(|&x| x as f32).collect();
            analyze(&rec, &starts, &s, self.rate)
        }
    }

    /// AC-16: 5–200 ms residuals at 48 kHz, −144 samples and 1 000 samples at 44.1 kHz, three seeds
    /// each, −20 dB loopback under −40 dBFS noise: accepted, error ≤ 1 sample.
    #[test]
    fn measures_loopback_residuals_within_one_sample() {
        let cases: [(u32, i64); 7] = [
            (48_000, 240),
            (48_000, 831),
            (48_000, 2_400),
            (48_000, 5_923),
            (48_000, 9_600),
            (48_000, -144),
            (44_100, 1_000),
        ];
        for (rate, delay) in cases {
            for seed in 1..=3 {
                let mut run = Run::new(rate, delay);
                run.seed = seed;
                let r = run.analyze();
                assert!(r.accepted, "{rate} Hz, delay {delay}, seed {seed}: {r:?}");
                assert!(r.confidence >= MIN_CONFIDENCE);
                assert!(
                    (r.offset_samples - delay as f64).abs() <= 1.0,
                    "{rate} Hz delay {delay} seed {seed}: measured {}",
                    r.offset_samples
                );
                assert!(
                    (r.offset_ms - delay as f64 / f64::from(rate) * 1000.0).abs() < 0.03,
                    "{}",
                    r.offset_ms
                );
                assert!(!r.clipped && !r.weak, "{r:?}");
            }
        }
    }

    /// 2nd-order Butterworth HPF 200 Hz + LPF 5 kHz (RBJ, Q = 1/√2), f64.
    fn bandpass(x: &[f64], rate: u32) -> Vec<f64> {
        fn biquad(x: &[f64], b: [f64; 3], a: [f64; 3]) -> Vec<f64> {
            let (mut x1, mut x2, mut y1, mut y2) = (0.0, 0.0, 0.0, 0.0);
            x.iter()
                .map(|&v| {
                    let y = (b[0] * v + b[1] * x1 + b[2] * x2 - a[1] * y1 - a[2] * y2) / a[0];
                    x2 = x1;
                    x1 = v;
                    y2 = y1;
                    y1 = y;
                    y
                })
                .collect()
        }
        let fs = f64::from(rate);
        let q = std::f64::consts::FRAC_1_SQRT_2;
        let hp = {
            let w = std::f64::consts::TAU * 200.0 / fs;
            let (c, al) = (w.cos(), w.sin() / (2.0 * q));
            (
                [(1.0 + c) / 2.0, -(1.0 + c), (1.0 + c) / 2.0],
                [1.0 + al, -2.0 * c, 1.0 - al],
            )
        };
        let lp = {
            let w = std::f64::consts::TAU * 5_000.0 / fs;
            let (c, al) = (w.cos(), w.sin() / (2.0 * q));
            (
                [(1.0 - c) / 2.0, 1.0 - c, (1.0 - c) / 2.0],
                [1.0 + al, -2.0 * c, 1.0 - al],
            )
        };
        biquad(&biquad(x, hp.0, hp.1), lp.0, lp.1)
    }

    fn bandpass_with_echo(x: &[f64], rate: u32) -> Vec<f64> {
        let d = (0.003 * f64::from(rate)).round() as usize;
        let g = 10f64.powf(-10.0 / 20.0);
        let mut y = x.to_vec();
        for i in d..x.len() {
            y[i] += g * x[i - d];
        }
        bandpass(&y, rate)
    }

    /// AC-16: a band-limited speaker→mic path with a −10 dB echo at +3 ms is accepted within
    /// ±0.25 ms of the residual and picks the direct path.
    #[test]
    fn band_limited_path_with_an_echo_stays_within_a_quarter_millisecond() {
        let mut run = Run::new(48_000, 2_400);
        run.path = bandpass_with_echo;
        let r = run.analyze();
        assert!(r.accepted, "{r:?}");
        assert!(
            (r.offset_ms - 50.0).abs() <= 0.25,
            "measured {} ms",
            r.offset_ms
        );
    }

    /// AC-17: digital silence → rejected, reason "no signal", confidence 0.
    #[test]
    fn silence_is_rejected_as_no_signal() {
        let mut run = Run::new(48_000, 240);
        run.gain_db = f64::NEG_INFINITY;
        run.noise_rms_dbfs = None;
        let r = run.analyze();
        assert!(!r.accepted);
        assert_eq!(r.reject, Some(CalibrationReject::NoSignal));
        assert_eq!(r.reps_agreeing, 0, "confidence 0");
        assert!(
            r.peak_dbfs.is_infinite() && r.peak_dbfs < 0.0,
            "digital silence"
        );
    }

    /// AC-17: white noise only at −20 dBFS → confidence < 0.8, rejected.
    #[test]
    fn noise_only_is_rejected() {
        for seed in 1..=3 {
            let mut run = Run::new(48_000, 240);
            run.gain_db = f64::NEG_INFINITY;
            run.noise_rms_dbfs = Some(-20.0);
            run.seed = seed;
            let r = run.analyze();
            assert!(!r.accepted, "seed {seed}: {r:?}");
            assert!(r.confidence < MIN_CONFIDENCE);
            assert_eq!(r.reject, Some(CalibrationReject::LowConfidence));
        }
    }

    /// AC-17: 3 of 5 repetitions muted → confidence 0.4 (rejected); 1 of 5 → 0.8 (accepted,
    /// correct value); loopback gain −40 dB accepted; −60 dB under −40 dBFS noise rejected.
    #[test]
    fn muted_repetitions_and_weak_loopback_follow_the_confidence_rule() {
        let mut three = Run::new(48_000, 831);
        three.mute = [true, false, true, false, true];
        let r = three.analyze();
        assert!(!r.accepted, "{r:?}");
        assert!((r.confidence - 0.4).abs() < 1e-9, "{}", r.confidence);

        let mut one = Run::new(48_000, 831);
        one.mute = [false, false, true, false, false];
        let r = one.analyze();
        assert!(r.accepted, "{r:?}");
        assert!((r.confidence - 0.8).abs() < 1e-9);
        assert!((r.offset_samples - 831.0).abs() <= 1.0);

        let mut quiet = Run::new(48_000, 831);
        quiet.gain_db = -40.0;
        let r = quiet.analyze();
        assert!(r.accepted, "{r:?}");
        assert!((r.offset_samples - 831.0).abs() <= 1.0);

        let mut too_quiet = Run::new(48_000, 831);
        too_quiet.gain_db = -60.0;
        let r = too_quiet.analyze();
        assert!(!r.accepted, "{r:?}");
    }

    /// AC-17: a loopback that clips produces the clipping warning (still measured).
    #[test]
    fn a_clipping_loopback_warns() {
        let mut run = Run::new(48_000, 240);
        run.gain_db = 18.0; // −12 dBFS sweep peak + 18 dB ⇒ ≥ 0 dBFS
        run.noise_rms_dbfs = None;
        run.path = |x, _| x.iter().map(|v| v.clamp(-1.0, 1.0)).collect();
        let r = run.analyze();
        assert!(r.clipped, "{r:?}");
    }

    /// §4.7: the sweep's length, peak level and fades.
    #[test]
    fn sweep_has_the_specified_shape() {
        let s = sweep(48_000);
        assert_eq!(s.len(), 48_000);
        let peak = s.iter().fold(0.0f32, |m, x| m.max(x.abs()));
        let peak_db = 20.0 * f64::from(peak).log10();
        assert!((peak_db - SWEEP_PEAK_DBFS).abs() < 0.05, "{peak_db}");
        assert_eq!(s[0].to_bits(), 0.0f32.to_bits(), "the fade-in starts at 0");
        assert!(s[s.len() - 1].abs() < 1e-3);
        assert!((sweep_f1_hz(22_050) - 9_922.5).abs() < 1e-9);
        assert_eq!(spacing_frames(48_000), 76_800);
    }
}

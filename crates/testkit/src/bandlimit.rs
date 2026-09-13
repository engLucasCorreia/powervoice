//! Band-limiting helpers for true-peak tests (SPEC-017 §5 stress set): Kaiser-windowed-sinc
//! low-pass, fades with silence padding, band-limited click trains.

use crate::error::TestkitError;
use crate::prng::Pcg32;

/// Modified Bessel function of the first kind, order 0 (power series).
pub(crate) fn bessel_i0(x: f64) -> f64 {
    let half = x / 2.0;
    let mut term = 1.0;
    let mut sum = 1.0;
    let mut k = 1.0;
    while term > 1e-17 * sum {
        term *= (half / k) * (half / k);
        sum += term;
        k += 1.0;
    }
    sum
}

/// Kaiser window at `x ∈ [−1, 1]` (0 outside).
pub(crate) fn kaiser(x: f64, beta: f64) -> f64 {
    if x.abs() >= 1.0 {
        return 0.0;
    }
    bessel_i0(beta * (1.0 - x * x).sqrt()) / bessel_i0(beta)
}

/// Normalized sinc, `sin(πx)/(πx)`.
pub(crate) fn sinc(x: f64) -> f64 {
    if x == 0.0 {
        1.0
    } else {
        let px = std::f64::consts::PI * x;
        px.sin() / px
    }
}

/// Kaiser's β for a stopband attenuation of `atten_db`.
pub fn kaiser_beta(atten_db: f64) -> f64 {
    if atten_db > 50.0 {
        0.1102 * (atten_db - 8.7)
    } else if atten_db >= 21.0 {
        0.5842 * (atten_db - 21.0).powf(0.4) + 0.07886 * (atten_db - 21.0)
    } else {
        0.0
    }
}

/// Zero-phase linear-phase low-pass (Kaiser-windowed sinc): −6 dB at `cutoff_hz`, passband up
/// to `cutoff_hz − transition_hz/2`, at least `atten_db` down from `cutoff_hz + transition_hz/2`.
/// The output has the input's length and alignment (the filter delay is compensated; the input
/// is zero-extended).
///
/// # Errors
/// `InvalidParameter` for a non-positive rate, a cut-off outside (0, fs/2) or a non-positive
/// transition width.
pub fn kaiser_lowpass(
    samples: &[f32],
    cutoff_hz: f64,
    transition_hz: f64,
    atten_db: f64,
    sample_rate: u32,
) -> Result<Vec<f32>, TestkitError> {
    let fs = f64::from(sample_rate);
    if sample_rate == 0 || !(cutoff_hz > 0.0 && cutoff_hz < fs / 2.0) || transition_hz <= 0.0 {
        return Err(TestkitError::InvalidParameter(format!(
            "kaiser_lowpass: cutoff {cutoff_hz} Hz, transition {transition_hz} Hz at {sample_rate} Hz"
        )));
    }
    let beta = kaiser_beta(atten_db);
    let order = ((atten_db - 7.95) / (14.36 * transition_hz / fs))
        .ceil()
        .max(2.0) as usize;
    let half = order.div_ceil(2); // odd length 2·half + 1, integer delay `half`
    let fc = cutoff_hz / fs; // cycles per sample
    let h: Vec<f64> = (0..=2 * half)
        .map(|i| {
            let k = i as f64 - half as f64;
            2.0 * fc * sinc(2.0 * fc * k) * kaiser(k / (half as f64 + 1.0), beta)
        })
        .collect();
    let dc: f64 = h.iter().sum();
    let n = samples.len();
    Ok((0..n)
        .map(|out| {
            // y[out] = Σ_k h[k] · x[out + half − k]
            let lo = (out + half).saturating_sub(n - 1);
            let hi = (out + half).min(2 * half);
            let mut acc = 0.0;
            for (k, hk) in h.iter().enumerate().take(hi + 1).skip(lo) {
                acc += hk * f64::from(samples[out + half - k]);
            }
            (acc / dc) as f32
        })
        .collect())
}

/// Raised-cosine fade-in and fade-out of `fade_ms` each, then `pad_ms` of digital silence
/// before and after.
pub fn fade_and_pad(samples: &[f32], fade_ms: f64, pad_ms: f64, sample_rate: u32) -> Vec<f32> {
    let fs = f64::from(sample_rate);
    let fade = ((fade_ms / 1000.0 * fs).round() as usize).min(samples.len() / 2);
    let pad = (pad_ms / 1000.0 * fs).round() as usize;
    let n = samples.len();
    let mut out = vec![0.0f32; pad];
    out.extend(samples.iter().enumerate().map(|(i, &x)| {
        let edge = i.min(n - 1 - i);
        if edge < fade {
            let w = 0.5 - 0.5 * (std::f64::consts::PI * (edge as f64 + 0.5) / fade as f64).cos();
            (f64::from(x) * w) as f32
        } else {
            x
        }
    }));
    out.extend(std::iter::repeat_n(0.0, pad));
    out
}

/// `count` band-limited clicks over `duration_s`: windowed-sinc pulses with cut-off
/// `cutoff_hz` (Kaiser, half-width 64 samples, β 8) centred at random sub-sample positions,
/// with random signs and peaks uniform in `[peak_min, peak_max]` (before overlap), then
/// low-passed with [`kaiser_lowpass`] (`cutoff_hz`, 1 kHz transition, 110 dB) so the content
/// stays below `cutoff_hz + 500 Hz`. Deterministic for `seed`.
///
/// # Errors
/// As [`kaiser_lowpass`], or a duration too short for the pulses.
pub fn bandlimited_clicks(
    seed: u64,
    count: usize,
    peak_min: f64,
    peak_max: f64,
    cutoff_hz: f64,
    duration_s: f64,
    sample_rate: u32,
) -> Result<Vec<f32>, TestkitError> {
    const HALF: usize = 64;
    let fs = f64::from(sample_rate);
    let n = (duration_s * fs).round() as usize;
    if n <= 4 * HALF {
        return Err(TestkitError::InvalidParameter(format!(
            "bandlimited_clicks: {duration_s} s is too short"
        )));
    }
    let fc = cutoff_hz / fs;
    // Pulse peak normalization: the continuous kernel peaks at 2·fc (sinc(0) · window(0)).
    let mut x = vec![0.0f64; n];
    let mut rng = Pcg32::new(seed, 3);
    for _ in 0..count {
        let centre = HALF as f64 + rng.next_f64() * (n - 2 * HALF - 1) as f64;
        let amp = (peak_min + (peak_max - peak_min) * rng.next_f64())
            * if rng.next_f64() < 0.5 { -1.0 } else { 1.0 };
        let first = centre.ceil() as usize - HALF;
        for (i, xi) in x.iter_mut().enumerate().skip(first).take(2 * HALF) {
            let t = i as f64 - centre;
            *xi += amp * sinc(2.0 * fc * t) * kaiser(t / HALF as f64, 8.0);
        }
    }
    let raw: Vec<f32> = x.iter().map(|&v| v as f32).collect();
    kaiser_lowpass(&raw, cutoff_hz, 1_000.0, 110.0, sample_rate)
}

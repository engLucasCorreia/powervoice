//! Fundamental-frequency (F0) estimation — YIN (de Cheveigné & Kawahara 2002, JASA 111(4)),
//! with the difference function computed through an FFT cross-correlation (H-42, SPEC-007
//! §8.6). Descriptive only: the diagnostics show the estimate as a note and Hz, never a
//! voice-type label.
//!
//! Per frame of `W + τ_max` samples:
//! 1. `d(τ) = Σ_{j<W} (x_j − x_{j+τ})² = e₀ + e_τ − 2·r(τ)` (energies from prefix sums, `r` from
//!    one forward FFT of each operand and one inverse FFT — O(N log N) instead of O(W·τ_max));
//! 2. cumulative-mean-normalized `d'(τ) = d(τ)·τ / Σ_{k=1..τ} d(k)`, `d'(0) = 1`;
//! 3. the first `τ ∈ [τ_min, τ_max]` where `d'` dips below [`YIN_THRESHOLD`], followed down to its
//!    local minimum (step 4 of the paper) — or the global minimum when nothing dips that low;
//! 4. parabolic interpolation of `d'` around that `τ`; `F0 = fs / τ*`.
//!
//! The frame is **unvoiced** (`None`) when its RMS is below [`YIN_MIN_RMS_DBFS`] or the chosen
//! dip's aperiodicity `d'(τ)` exceeds [`YIN_MAX_APERIODICITY`].
//!
//! Range [`F0_MIN_HZ`] … [`F0_MAX_HZ`] covers speaking and singing voices; `W = τ_max` (one
//! period of the lowest F0, 20 ms).

use std::sync::Arc;

use realfft::num_complex::Complex;
use realfft::{ComplexToReal, RealFftPlanner, RealToComplex};

/// Lowest F0 searched, Hz.
pub const F0_MIN_HZ: f64 = 50.0;
/// Highest F0 searched, Hz.
pub const F0_MAX_HZ: f64 = 1000.0;
/// YIN absolute threshold (the paper uses 0.1–0.15).
pub const YIN_THRESHOLD: f64 = 0.15;
/// Above this aperiodicity the frame counts as unvoiced.
pub const YIN_MAX_APERIODICITY: f64 = 0.35;
/// Frames quieter than this (RMS, dBFS) are unvoiced without looking further.
pub const YIN_MIN_RMS_DBFS: f64 = -60.0;

/// One voiced estimate.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pitch {
    pub f0_hz: f64,
    /// `d'(τ*)`: 0 = perfectly periodic, larger = noisier.
    pub aperiodicity: f64,
}

/// A reusable YIN estimator for one sample rate. Allocation-free after construction.
pub struct Yin {
    sample_rate_hz: f64,
    tau_min: usize,
    tau_max: usize,
    w: usize,
    fwd: Arc<dyn RealToComplex<f64>>,
    inv: Arc<dyn ComplexToReal<f64>>,
    n: usize,
    a: Vec<f64>,
    b: Vec<f64>,
    spec_a: Vec<Complex<f64>>,
    spec_b: Vec<Complex<f64>>,
    scratch_fwd: Vec<Complex<f64>>,
    scratch_inv: Vec<Complex<f64>>,
    corr: Vec<f64>,
    prefix: Vec<f64>,
    cmnd: Vec<f64>,
    last_had_signal: bool,
}

impl Yin {
    pub fn new(sample_rate_hz: u32) -> Self {
        let fs = f64::from(sample_rate_hz.max(1));
        let tau_max = (fs / F0_MIN_HZ).ceil() as usize;
        let tau_min = ((fs / F0_MAX_HZ).floor() as usize).max(2);
        let w = tau_max;
        let frame_len = w + tau_max + 1;
        let n = frame_len.next_power_of_two();
        let mut planner = RealFftPlanner::<f64>::new();
        let fwd = planner.plan_fft_forward(n);
        let inv = planner.plan_fft_inverse(n);
        Self {
            sample_rate_hz: fs,
            tau_min,
            tau_max,
            w,
            a: fwd.make_input_vec(),
            b: fwd.make_input_vec(),
            spec_a: fwd.make_output_vec(),
            spec_b: fwd.make_output_vec(),
            scratch_fwd: vec![Complex::default(); fwd.get_scratch_len()],
            scratch_inv: vec![Complex::default(); inv.get_scratch_len()],
            corr: inv.make_output_vec(),
            prefix: vec![0.0; frame_len + 1],
            cmnd: vec![0.0; tau_max + 2],
            last_had_signal: false,
            fwd,
            inv,
            n,
        }
    }

    /// Whether the last [`Self::estimate`] call saw a frame above the energy gate (voiced or
    /// not) — the denominator of the voiced fraction.
    pub fn last_had_signal(&self) -> bool {
        self.last_had_signal
    }

    /// Samples one estimate looks at (`W + τ_max + 1`; 1 921 at 48 kHz).
    pub fn frame_len(&self) -> usize {
        self.w + self.tau_max + 1
    }

    /// Estimates F0 from the **last** [`Self::frame_len`] samples of `frame` (fewer samples →
    /// `None`). Non-finite samples count as 0.
    pub fn estimate(&mut self, frame: &[f32]) -> Option<Pitch> {
        let len = self.frame_len();
        self.last_had_signal = false;
        if frame.len() < len {
            return None;
        }
        let x = &frame[frame.len() - len..];

        // Energy gate + prefix sums of x².
        self.prefix[0] = 0.0;
        for (i, &s) in x.iter().enumerate() {
            let v = if s.is_finite() { f64::from(s) } else { 0.0 };
            self.prefix[i + 1] = self.prefix[i] + v * v;
        }
        let mean_sq = self.prefix[len] / len as f64;
        if mean_sq <= 0.0 || 10.0 * mean_sq.log10() < YIN_MIN_RMS_DBFS {
            return None;
        }
        self.last_had_signal = true;

        // r(τ) = Σ_{j<W} x_j x_{j+τ} via conj(FFT(a))·FFT(b); b holds the whole frame, a its
        // first W samples, both zero-padded to n ≥ len — no circular wrap for τ ≤ τ_max.
        self.a.fill(0.0);
        self.b.fill(0.0);
        for (i, &s) in x.iter().enumerate() {
            let v = if s.is_finite() { f64::from(s) } else { 0.0 };
            self.b[i] = v;
            if i < self.w {
                self.a[i] = v;
            }
        }
        let ok_a = self
            .fwd
            .process_with_scratch(&mut self.a, &mut self.spec_a, &mut self.scratch_fwd)
            .is_ok();
        let ok_b = self
            .fwd
            .process_with_scratch(&mut self.b, &mut self.spec_b, &mut self.scratch_fwd)
            .is_ok();
        debug_assert!(ok_a && ok_b);
        for (sa, sb) in self.spec_a.iter_mut().zip(&self.spec_b) {
            *sa = sa.conj() * sb;
        }
        // The DC and Nyquist bins of a real signal's spectrum must be real for the inverse.
        if let Some(first) = self.spec_a.first_mut() {
            first.im = 0.0;
        }
        if let Some(last) = self.spec_a.last_mut() {
            last.im = 0.0;
        }
        let ok_inv = self
            .inv
            .process_with_scratch(&mut self.spec_a, &mut self.corr, &mut self.scratch_inv)
            .is_ok();
        debug_assert!(ok_inv);
        let scale = 1.0 / self.n as f64;

        // d'(τ) for τ = 0 … τ_max + 1.
        let e0 = self.prefix[self.w];
        let mut running = 0.0;
        self.cmnd[0] = 1.0;
        for tau in 1..=self.tau_max + 1 {
            let e_tau = self.prefix[tau + self.w] - self.prefix[tau];
            let d = (e0 + e_tau - 2.0 * self.corr[tau] * scale).max(0.0);
            running += d;
            self.cmnd[tau] = if running > 0.0 {
                d * tau as f64 / running
            } else {
                1.0
            };
        }

        let tau = self.pick_tau()?;
        let (tau_star, aperiodicity) = self.refine(tau);
        if aperiodicity > YIN_MAX_APERIODICITY || tau_star <= 0.0 {
            return None;
        }
        Some(Pitch {
            f0_hz: self.sample_rate_hz / tau_star,
            aperiodicity,
        })
    }

    fn pick_tau(&self) -> Option<usize> {
        let mut tau = self.tau_min;
        while tau <= self.tau_max {
            if self.cmnd[tau] < YIN_THRESHOLD {
                while tau < self.tau_max && self.cmnd[tau + 1] < self.cmnd[tau] {
                    tau += 1;
                }
                return Some(tau);
            }
            tau += 1;
        }
        (self.tau_min..=self.tau_max).min_by(|&a, &b| self.cmnd[a].total_cmp(&self.cmnd[b]))
    }

    /// Parabolic interpolation of `d'` around `tau` → (`τ*`, `d'(τ*)`).
    fn refine(&self, tau: usize) -> (f64, f64) {
        let y0 = self.cmnd[tau - 1];
        let y1 = self.cmnd[tau];
        let y2 = self.cmnd[tau + 1];
        let denom = y0 - 2.0 * y1 + y2;
        if denom.abs() < 1e-12 {
            return (tau as f64, y1);
        }
        let delta = (0.5 * (y0 - y2) / denom).clamp(-1.0, 1.0);
        let value = y1 - 0.25 * (y0 - y2) * delta;
        (tau as f64 + delta, value.max(0.0))
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    const FS: u32 = 48_000;

    fn harmonic_series(f0: f64, harmonics: &[(u32, f64)], len: usize, fs: u32) -> Vec<f32> {
        (0..len)
            .map(|i| {
                let t = i as f64 / f64::from(fs);
                harmonics
                    .iter()
                    .map(|&(h, a)| a * (std::f64::consts::TAU * f0 * f64::from(h) * t).sin())
                    .sum::<f64>() as f32
            })
            .collect()
    }

    fn cents(a: f64, b: f64) -> f64 {
        1200.0 * (a / b).log2()
    }

    #[test]
    fn a_sine_reads_its_frequency() {
        for fs in [44_100, 48_000, 96_000] {
            let mut yin = Yin::new(fs);
            for f in [55.0, 110.0, 220.0, 440.0, 880.0] {
                let x = harmonic_series(f, &[(1, 0.5)], yin.frame_len(), fs);
                let p = yin.estimate(&x).expect("voiced");
                assert!(cents(p.f0_hz, f).abs() < 2.0, "{fs} Hz: {f} → {}", p.f0_hz);
                assert!(p.aperiodicity < 0.05);
            }
        }
    }

    #[test]
    fn a_missing_fundamental_is_still_found() {
        // Harmonics 2…8 of 200 Hz, no energy at 200 Hz itself: the waveform's period is still
        // 5 ms, which is what YIN measures (the ear hears 200 Hz too).
        let mut yin = Yin::new(FS);
        let hs: Vec<(u32, f64)> = (2..=8).map(|h| (h, 0.3 / f64::from(h))).collect();
        let x = harmonic_series(200.0, &hs, yin.frame_len(), FS);
        let p = yin.estimate(&x).expect("voiced");
        assert!(cents(p.f0_hz, 200.0).abs() < 3.0, "{}", p.f0_hz);
    }

    /// A crude voice: a glottal-like harmonic series (−12 dB/octave source tilt) at `f0`
    /// shaped by three formant resonances (/a/: 730, 1090, 2440 Hz), with slow vibrato.
    pub(crate) fn voice_like(f0: f64, seconds: f64, fs: u32) -> Vec<f32> {
        let formants = [(730.0, 80.0), (1090.0, 90.0), (2440.0, 120.0)];
        let gain = |f: f64| -> f64 {
            formants
                .iter()
                .map(|&(fc, bw)| {
                    let x = (f - fc) / (bw / 2.0);
                    1.0 / (1.0 + x * x).sqrt()
                })
                .sum::<f64>()
                + 0.05
        };
        let n = (seconds * f64::from(fs)) as usize;
        let mut phase = 0.0f64;
        let mut out = Vec::with_capacity(n);
        let max_h = ((f64::from(fs) / 2.0 * 0.9) / (f0 * 1.05)) as u32;
        for i in 0..n {
            let t = i as f64 / f64::from(fs);
            let inst = f0 * (1.0 + 0.02 * (std::f64::consts::TAU * 5.0 * t).sin());
            phase += std::f64::consts::TAU * inst / f64::from(fs);
            let mut s = 0.0;
            for h in 1..=max_h {
                let fh = inst * f64::from(h);
                s += gain(fh) / f64::from(h * h) * (phase * f64::from(h)).sin();
            }
            out.push((0.2 * s) as f32);
        }
        out
    }

    #[test]
    fn a_voice_like_signal_tracks_its_f0() {
        let mut yin = Yin::new(FS);
        let x = voice_like(130.0, 1.0, FS);
        let hop = FS as usize / 100;
        let mut estimates = Vec::new();
        let mut end = yin.frame_len();
        while end <= x.len() {
            if let Some(p) = yin.estimate(&x[..end]) {
                estimates.push(p.f0_hz);
            }
            end += hop;
        }
        assert!(estimates.len() > 80, "voiced frames: {}", estimates.len());
        estimates.sort_by(f64::total_cmp);
        let median = estimates[estimates.len() / 2];
        assert!(cents(median, 130.0).abs() < 25.0, "median {median}");
        // No octave errors: every frame within the ±2 % vibrato (+ margin).
        assert!(estimates.iter().all(|&f| cents(f, 130.0).abs() < 60.0));
    }

    #[test]
    fn silence_and_noise_are_unvoiced() {
        let mut yin = Yin::new(FS);
        assert!(yin.estimate(&vec![0.0; yin.frame_len()]).is_none());
        let noise = vox_testkit::signal::white_noise(3, -20.0, 0.1, FS).unwrap();
        assert!(yin.estimate(&noise).is_none());
        // Too short.
        assert!(yin.estimate(&[0.1; 100]).is_none());
        // Very quiet but periodic: below the energy gate.
        let x = harmonic_series(200.0, &[(1, 1e-4)], yin.frame_len(), FS);
        assert!(yin.estimate(&x).is_none());
    }
}

//! Live output spectrum analyzer math (SPEC-007 §4.8, ADR-003 `VXSA`): a one-shot 4-term
//! Blackman-Harris (BH4) windowed real FFT, reduction of bins to 1/24-octave bands, and
//! power-domain exponential averaging (Fast/Medium/Slow). Off the audio thread only (the
//! engine's analyzer consumer thread calls this, never the RT output callback, which only taps
//! samples into a ring); pure and deterministic.
//!
//! Conventions (normative, SPEC-007 §4.2, §4.8):
//! - BH4 window `w[n] = a0 − a1·cos(2πn/N) + a2·cos(4πn/N) − a3·cos(6πn/N)` (periodic; SPEC-012
//!   §4.3 coefficients: `a0 = 0.35875, a1 = 0.48829, a2 = 0.14128, a3 = 0.01168`).
//! - Bin power `P[k] = |X[k]|² / (Σw/2)²`, so a full-scale sine centred on a bin reads 0 dB.
//! - Band centres `f_k = 20·2^(k/24)` Hz, edges `f_k·2^(±1/48)`. Band power is the **maximum**
//!   bin power with centre in `[lo, hi)`; when no bin centre falls inside, it is the dB-linear
//!   interpolation of the two bins bracketing `f_k`.
//! - Averaging is in the **power** domain: `P ← P + α·(p − P)`, `α = 1 − exp(−(1/rate)/τ)`,
//!   `τ ∈ {50, 150, 500} ms` for Fast/Medium/Slow. `L = 10·log10(P)`, and `0 → −∞` (never NaN).
//! - Non-finite input samples are analysed as 0.0 (SPEC-007 AC-3 convention).

use std::sync::Arc;

use realfft::num_complex::Complex;
use realfft::{RealFftPlanner, RealToComplex};

/// BH4 window coefficients (SPEC-012 §4.3), reused here per SPEC-007 §4.8.2 for the analyzer's
/// fixed window (not user-selectable, unlike the spectrogram's Hann).
const BH4_A0: f64 = 0.35875;
const BH4_A1: f64 = 0.48829;
const BH4_A2: f64 = 0.14128;
const BH4_A3: f64 = 0.01168;

/// Analyzer band-centre base frequency (SPEC-007 §4.8.3, ADR-003 `VXSA`).
pub const F0_HZ: f64 = 20.0;
/// Analyzer band resolution (SPEC-007 §4.8.3, ADR-003 `VXSA`).
pub const BANDS_PER_OCTAVE: u32 = 24;
/// Smallest and largest FFT sizes the analyzer offers (SPEC-007 §3 `an_fft_size`).
pub const MIN_FFT_SIZE: u32 = 4096;
pub const MAX_FFT_SIZE: u32 = 32_768;

/// SPEC-007 §3 `an_fft_size`: the power of two nearest to `8192 · fs / 48 000`, clamped to
/// 4096 ..= 32768 (8192 at 44.1/48 kHz, 16384 at 88.2/96 kHz, 32768 at 176.4/192 kHz).
pub fn auto_fft_size(sample_rate_hz: u32) -> u32 {
    let target = 8192.0 * f64::from(sample_rate_hz) / 48_000.0;
    let mut best = MIN_FFT_SIZE;
    let mut n = MIN_FFT_SIZE;
    while n <= MAX_FFT_SIZE {
        if (f64::from(n) - target).abs() < (f64::from(best) - target).abs() {
            best = n;
        }
        n *= 2;
    }
    best
}

/// Analyzer band-centre frequency `f_k = f0 · 2^(k / bands_per_octave)` Hz (SPEC-007 §4.8.3).
pub fn band_center_hz(k: u32) -> f64 {
    F0_HZ * 2f64.powf(f64::from(k) / f64::from(BANDS_PER_OCTAVE))
}

/// Number of bands `K` such that `f_{K-1} ≤ min(fs/2, 24 000)` (SPEC-007 §4.8.3): 246 at 48 kHz,
/// 243 at 44.1 kHz.
pub fn band_count(sample_rate_hz: u32) -> u32 {
    let f_max = (f64::from(sample_rate_hz) / 2.0).min(24_000.0);
    let mut k = 0u32;
    while band_center_hz(k) <= f_max {
        k += 1;
    }
    k
}

/// One-shot BH4-windowed real FFT of `fft_size` points, producing sine-normalized bin powers.
/// Reused across frames; allocation-free after construction.
struct Bh4Fft {
    fft: Arc<dyn RealToComplex<f32>>,
    window: Box<[f32]>,
    frame: Vec<f32>,
    spectrum: Vec<Complex<f32>>,
    scratch: Vec<Complex<f32>>,
    /// `1 / (Σw/2)²`.
    inv_norm_sq: f64,
}

impl Bh4Fft {
    /// # Panics
    /// If `fft_size` is not a power of two ≥ 4.
    fn new(fft_size: usize) -> Self {
        assert!(
            fft_size.is_power_of_two() && fft_size >= 4,
            "FFT size must be a power of two ≥ 4"
        );
        let fft = RealFftPlanner::<f32>::new().plan_fft_forward(fft_size);
        let n = fft_size as f64;
        let window_f64: Vec<f64> = (0..fft_size)
            .map(|i| {
                let x = std::f64::consts::TAU * i as f64 / n;
                BH4_A0 - BH4_A1 * x.cos() + BH4_A2 * (2.0 * x).cos() - BH4_A3 * (3.0 * x).cos()
            })
            .collect();
        let half_sum = window_f64.iter().sum::<f64>() / 2.0;
        Self {
            scratch: vec![Complex::default(); fft.get_scratch_len()],
            frame: fft.make_input_vec(),
            spectrum: fft.make_output_vec(),
            window: window_f64.iter().map(|&w| w as f32).collect(),
            inv_norm_sq: 1.0 / (half_sum * half_sum),
            fft,
        }
    }

    fn fft_size(&self) -> usize {
        self.window.len()
    }

    fn bins(&self) -> usize {
        self.spectrum.len()
    }

    /// Sine-normalized bin powers `|X[k]|² / (Σw/2)²` of `input` (length `fft_size`) into `out`
    /// (length `bins()`). Non-finite samples count as 0.
    fn power_spectrum(&mut self, input: &[f32], out: &mut [f64]) {
        debug_assert_eq!(input.len(), self.window.len());
        debug_assert_eq!(out.len(), self.spectrum.len());
        for ((dst, &x), &w) in self.frame.iter_mut().zip(input).zip(self.window.iter()) {
            *dst = if x.is_finite() { x * w } else { 0.0 };
        }
        // Lengths match the plan by construction, so this cannot fail.
        let ok = self
            .fft
            .process_with_scratch(&mut self.frame, &mut self.spectrum, &mut self.scratch)
            .is_ok();
        debug_assert!(ok);
        for (o, c) in out.iter_mut().zip(self.spectrum.iter()) {
            let re = f64::from(c.re);
            let im = f64::from(c.im);
            *o = (re * re + im * im) * self.inv_norm_sq;
        }
    }
}

/// How one band reduces the bin-power spectrum to a single power value (SPEC-007 §4.8.3).
#[derive(Debug, Clone)]
enum BandMap {
    /// The maximum power among bins `[start, end)` whose centre falls in `[lo, hi)`.
    Max(std::ops::Range<usize>),
    /// No bin centre falls in `[lo, hi)`: dB-linear interpolation between these two bins
    /// bracketing the band centre, at weight `t` toward the second.
    Interp(usize, usize, f64),
}

fn interp_power(p0: f64, p1: f64, t: f64) -> f64 {
    if p0 <= 0.0 && p1 <= 0.0 {
        return 0.0;
    }
    // A finite floor keeps the interpolation well-defined when exactly one side is silent;
    // both-silent (the common digital-silence case) is handled exactly above.
    const FLOOR_DB: f64 = -300.0;
    let d0 = if p0 > 0.0 {
        10.0 * p0.log10()
    } else {
        FLOOR_DB
    };
    let d1 = if p1 > 0.0 {
        10.0 * p1.log10()
    } else {
        FLOOR_DB
    };
    let d = d0 + (d1 - d0) * t;
    10f64.powf(d / 10.0)
}

/// Precomputed band → bin mapping for one `(fft_size, sample_rate_hz)` pair. Built once, off
/// the audio thread; [`BandTable::reduce`] is then allocation-free.
pub struct BandTable {
    map: Vec<BandMap>,
}

impl BandTable {
    pub fn new(fft_size: u32, sample_rate_hz: u32) -> Self {
        let bins = fft_size as usize / 2 + 1;
        let bin_hz = |b: usize| b as f64 * f64::from(sample_rate_hz) / f64::from(fft_size);
        let k = band_count(sample_rate_hz);
        let map = (0..k)
            .map(|band| {
                let center = band_center_hz(band);
                let lo = center * 2f64.powf(-1.0 / 48.0);
                let hi = center * 2f64.powf(1.0 / 48.0);
                let mut start = None;
                let mut end = 0usize;
                for b in 0..bins {
                    let f = bin_hz(b);
                    if f >= lo && f < hi {
                        if start.is_none() {
                            start = Some(b);
                        }
                        end = b + 1;
                    } else if f >= hi {
                        break;
                    }
                }
                if let Some(start) = start {
                    BandMap::Max(start..end)
                } else {
                    let bin_pos = (center * f64::from(fft_size) / f64::from(sample_rate_hz))
                        .clamp(0.0, (bins - 1) as f64);
                    let b0 = bin_pos.floor() as usize;
                    let b1 = (bin_pos.ceil() as usize).min(bins - 1);
                    BandMap::Interp(b0, b1, bin_pos - b0 as f64)
                }
            })
            .collect();
        Self { map }
    }

    pub fn band_count(&self) -> u32 {
        self.map.len() as u32
    }

    /// Reduces bin powers to band powers (linear power domain; the `p` fed to the EMA).
    pub fn reduce(&self, bin_power: &[f64], out: &mut [f64]) {
        debug_assert_eq!(out.len(), self.map.len());
        for (o, m) in out.iter_mut().zip(&self.map) {
            *o = match m {
                BandMap::Max(range) => range.clone().map(|b| bin_power[b]).fold(0.0_f64, f64::max),
                BandMap::Interp(b0, b1, t) => interp_power(bin_power[*b0], bin_power[*b1], *t),
            };
        }
    }
}

/// Averaging response (SPEC-007 §2.9/§4.8.4): exponential averaging in the power domain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Response {
    Fast,
    Medium,
    Slow,
}

impl Response {
    /// Time constant τ (ADR-003 `VXSA` / SPEC-007 §3 `an_response`).
    pub fn tau_s(self) -> f64 {
        match self {
            Response::Fast => 0.050,
            Response::Medium => 0.150,
            Response::Slow => 0.500,
        }
    }

    /// Wire code (ADR-003 amendment: 0 fast, 1 medium, 2 slow).
    pub fn code(self) -> u32 {
        match self {
            Response::Fast => 0,
            Response::Medium => 1,
            Response::Slow => 2,
        }
    }

    pub fn from_code(code: u32) -> Option<Self> {
        match code {
            0 => Some(Response::Fast),
            1 => Some(Response::Medium),
            2 => Some(Response::Slow),
            _ => None,
        }
    }

    fn alpha(self, rate_hz: f64) -> f64 {
        1.0 - (-(1.0 / rate_hz) / self.tau_s()).exp()
    }
}

/// The live analyzer pipeline: one BH4 FFT + band reduction per call to [`Analyzer::process`],
/// feeding all three [`Response`] averages at once (SPEC-007 §4.8.2) so every subscriber reads
/// its own response from the **same** underlying computation (ADR-003 `VXSA`: "each subscriber
/// gets its own channel from one computation").
pub struct Analyzer {
    fft: Bh4Fft,
    bins: Vec<f64>,
    bands: BandTable,
    band_power: Vec<f64>,
    fast: Vec<f64>,
    medium: Vec<f64>,
    slow: Vec<f64>,
    alpha_fast: f64,
    alpha_medium: f64,
    alpha_slow: f64,
}

impl Analyzer {
    /// # Panics
    /// If `fft_size` is not a power of two ≥ 4.
    pub fn new(fft_size: u32, sample_rate_hz: u32, rate_hz: f64) -> Self {
        let fft = Bh4Fft::new(fft_size as usize);
        let bins = vec![0.0; fft.bins()];
        let bands = BandTable::new(fft_size, sample_rate_hz);
        let k = bands.band_count() as usize;
        Self {
            fft,
            bins,
            bands,
            band_power: vec![0.0; k],
            fast: vec![0.0; k],
            medium: vec![0.0; k],
            slow: vec![0.0; k],
            alpha_fast: Response::Fast.alpha(rate_hz),
            alpha_medium: Response::Medium.alpha(rate_hz),
            alpha_slow: Response::Slow.alpha(rate_hz),
        }
    }

    pub fn fft_size(&self) -> usize {
        self.fft.fft_size()
    }

    pub fn band_count(&self) -> u32 {
        self.bands.band_count()
    }

    /// Recomputes the EMA time constants for a new frame rate (`TELEMETRY_RATE` change).
    pub fn set_rate_hz(&mut self, rate_hz: f64) {
        self.alpha_fast = Response::Fast.alpha(rate_hz);
        self.alpha_medium = Response::Medium.alpha(rate_hz);
        self.alpha_slow = Response::Slow.alpha(rate_hz);
    }

    /// Clears all averaging state (SPEC-007 §4.8.6: device reopen / rate change).
    pub fn reset(&mut self) {
        self.fast.fill(0.0);
        self.medium.fill(0.0);
        self.slow.fill(0.0);
    }

    /// Processes one `fft_size`-sample analysis window (the latest samples of the tap history),
    /// updating all three response states. Returns `true` if the window is digital silence
    /// (every sample exactly 0.0).
    pub fn process(&mut self, window: &[f32]) -> bool {
        debug_assert_eq!(window.len(), self.fft.fft_size());
        self.fft.power_spectrum(window, &mut self.bins);
        self.bands.reduce(&self.bins, &mut self.band_power);
        for i in 0..self.band_power.len() {
            let p = self.band_power[i];
            self.fast[i] += self.alpha_fast * (p - self.fast[i]);
            self.medium[i] += self.alpha_medium * (p - self.medium[i]);
            self.slow[i] += self.alpha_slow * (p - self.slow[i]);
        }
        window.iter().all(|&s| s == 0.0)
    }

    /// Averaged band levels in dB for one response (`out.len() == band_count()`); `−∞` allowed,
    /// never NaN.
    pub fn levels_db(&self, response: Response, out: &mut [f32]) {
        let state = match response {
            Response::Fast => &self.fast,
            Response::Medium => &self.medium,
            Response::Slow => &self.slow,
        };
        debug_assert_eq!(out.len(), state.len());
        for (o, &p) in out.iter_mut().zip(state) {
            *o = if p > 0.0 {
                (10.0 * p.log10()) as f32
            } else {
                f32::NEG_INFINITY
            };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FS: u32 = 48_000;

    fn sine(freq_hz: f64, level_dbfs: f64, len: usize) -> Vec<f32> {
        let amp = 10f64.powf(level_dbfs / 20.0);
        let w = std::f64::consts::TAU * freq_hz / f64::from(FS);
        (0..len)
            .map(|i| (amp * (w * i as f64).sin()) as f32)
            .collect()
    }

    // --- AC-19-adjacent: FFT size / band table shape ---

    #[test]
    fn auto_fft_size_defaults() {
        assert_eq!(auto_fft_size(48_000), 8192);
        assert_eq!(auto_fft_size(44_100), 8192);
        assert_eq!(auto_fft_size(96_000), 16_384);
        assert_eq!(auto_fft_size(88_200), 16_384);
        assert_eq!(auto_fft_size(192_000), 32_768);
        assert_eq!(auto_fft_size(176_400), 32_768);
        // Low rates clamp to the minimum.
        assert_eq!(auto_fft_size(22_050), 4096);
        assert_eq!(auto_fft_size(8_000), 4096);
    }

    #[test]
    fn band_count_matches_spec() {
        // SPEC-007 §4.8.3: K = 246 at 48 kHz, 243 at 44.1 kHz.
        assert_eq!(band_count(48_000), 246);
        assert_eq!(band_count(44_100), 243);
    }

    #[test]
    fn band_centers_follow_formula() {
        for k in [0u32, 1, 50, 135, 245] {
            let expected = 20.0 * 2f64.powf(f64::from(k) / 24.0);
            assert!((band_center_hz(k) - expected).abs() < 1e-9);
        }
        // f_0 = 20 Hz exactly.
        assert!((band_center_hz(0) - 20.0).abs() < 1e-9);
    }

    fn nearest_band(freq_hz: f64, k_max: u32) -> u32 {
        (0..k_max)
            .min_by(|&a, &b| {
                (band_center_hz(a) - freq_hz)
                    .abs()
                    .partial_cmp(&(band_center_hz(b) - freq_hz).abs())
                    .unwrap()
            })
            .unwrap()
    }

    // --- AC-15: analyzer level and frequency accuracy ---

    #[test]
    fn ac15_bin_centered_tone_reads_0_2db_of_target() {
        // 1 001.953125 Hz = bin 171 of N = 8192 @ 48 kHz (bin-centred): reads −20.00 ± 0.2 dB.
        let n = 8192usize;
        let bin: u32 = 171;
        let freq = f64::from(bin) * f64::from(FS) / n as f64;
        assert!((freq - 1_001.953_125).abs() < 1e-6);

        let mut an = Analyzer::new(n as u32, FS, 60.0);
        let sig = sine(freq, -20.0, n * 4);
        // Settle the EMA over several frames of the steady tone.
        for _ in 0..200 {
            an.process(&sig[sig.len() - n..]);
        }
        let mut out = vec![0.0f32; an.band_count() as usize];
        an.levels_db(Response::Medium, &mut out);
        let band = nearest_band(freq, an.band_count());
        let max_band = (0..out.len())
            .max_by(|&a, &b| out[a].partial_cmp(&out[b]).unwrap())
            .unwrap();
        assert_eq!(
            max_band as u32, band,
            "the tone's band should be the loudest"
        );
        assert!(
            (out[band as usize] - (-20.0)).abs() < 0.2,
            "expected -20.00 ± 0.2 dB, got {}",
            out[band as usize]
        );
    }

    #[test]
    fn ac15_off_bin_1khz_tone_scalloping() {
        // 1000 Hz at 48 kHz / N = 8192: bin_pos = 170.667 (δ = 1/3 bin), BH4 scalloping reads
        // -0.37 dB below the tone level (SPEC-007 §4.8.5) => -20.37 dB for a -20 dBFS tone. The
        // tone sits almost exactly on a band edge (band centres are log-spaced, bins are
        // linear), so the loudest band may be either side of the "nearest centre" pick — assert
        // on the true maximum, and that it is adjacent to the expected band.
        let n = 8192usize;
        let mut an = Analyzer::new(n as u32, FS, 60.0);
        let sig = sine(1000.0, -20.0, n * 4);
        for _ in 0..200 {
            an.process(&sig[sig.len() - n..]);
        }
        let mut out = vec![0.0f32; an.band_count() as usize];
        an.levels_db(Response::Medium, &mut out);
        let band = nearest_band(1000.0, an.band_count());
        let max_band = (0..out.len())
            .max_by(|&a, &b| out[a].partial_cmp(&out[b]).unwrap())
            .unwrap();
        assert!(
            (max_band as i64 - i64::from(band)).abs() <= 1,
            "expected the loudest band adjacent to band {band} (1 kHz), got {max_band}"
        );
        assert!(
            (out[max_band] - (-20.37)).abs() < 0.2,
            "expected -20.37 ± 0.2 dB, got {}",
            out[max_band]
        );
    }

    #[test]
    fn full_scale_bin_centered_sine_reads_0db() {
        let n = 8192usize;
        let bin: u32 = 171;
        let freq = f64::from(bin) * f64::from(FS) / n as f64;
        let mut an = Analyzer::new(n as u32, FS, 60.0);
        let sig = sine(freq, 0.0, n * 4);
        for _ in 0..200 {
            an.process(&sig[sig.len() - n..]);
        }
        let mut out = vec![0.0f32; an.band_count() as usize];
        an.levels_db(Response::Medium, &mut out);
        let band = nearest_band(freq, an.band_count());
        assert!(
            (out[band as usize] - 0.0).abs() < 0.35,
            "got {}",
            out[band as usize]
        );
    }

    // --- SILENT / -inf, never NaN ---

    #[test]
    fn silence_gives_neg_infinity_never_nan() {
        let n = 4096usize;
        let mut an = Analyzer::new(n as u32, FS, 60.0);
        let zeros = vec![0.0f32; n];
        let silent = an.process(&zeros);
        assert!(silent);
        let mut out = vec![0.0f32; an.band_count() as usize];
        an.levels_db(Response::Fast, &mut out);
        for (i, &v) in out.iter().enumerate() {
            assert!(!v.is_nan(), "band {i} is NaN");
            assert!(
                v.is_infinite() && v.is_sign_negative(),
                "band {i} should be -inf, got {v}"
            );
        }
    }

    #[test]
    fn non_silent_window_is_reported() {
        let n = 4096usize;
        let mut an = Analyzer::new(n as u32, FS, 60.0);
        let sig = sine(1000.0, -20.0, n);
        assert!(!an.process(&sig));
    }

    // --- AC-16: response time calibration (SPEC-007 §4.8.8, N=8192, 48 kHz, 60 Hz frames) ---
    // Table: Fast 0.18s rise / 0.30s fall; Medium 0.33s / 0.77s; Slow 0.88s / 2.37s.
    // The acceptance thresholds (AC-16) are looser (0.25/0.40/1.0 rise; 0.40/0.85/2.6 fall).

    fn step_response(response: Response, rise_budget_s: f64, fall_budget_s: f64) {
        let n = 8192usize;
        let rate_hz = 60.0;
        let hop = (f64::from(FS) / rate_hz).round() as usize; // 800 samples/tick @ 48kHz/60Hz
        let band = nearest_band(1000.0, band_count(FS));
        let freq = band_center_hz(band); // bin/band-centred tone: unambiguous final value
        let level_dbfs = -20.0;

        // Silence, then the tone for a long time, then silence again.
        let pre_s = 1.0;
        let tone_s = 3.0;
        let post_s = 3.0;
        let pre_n = (pre_s * f64::from(FS)) as usize;
        let tone_n = (tone_s * f64::from(FS)) as usize;
        let post_n = (post_s * f64::from(FS)) as usize;

        let mut signal = vec![0.0f32; pre_n];
        signal.extend(sine(freq, level_dbfs, tone_n));
        signal.extend(vec![0.0f32; post_n]);

        let mut an = Analyzer::new(n as u32, FS, rate_hz);
        let mut history = vec![0.0f32; n];

        // Final steady-state level: the last reading while still inside the tone's final second
        // (hop may not divide the tone length evenly, so track the last in-range tick rather
        // than requiring an exact sample match).
        let steady_end = pre_n + tone_n; // last sample of the tone
        let steady_start = steady_end.saturating_sub(FS as usize); // last 1 s of the tone
        let mut final_db = f32::NAN;
        let mut center = n; // first window fully available
        while center <= signal.len() {
            let start = center - n;
            history.copy_from_slice(&signal[start..center]);
            an.process(&history);
            if center >= steady_start && center <= steady_end {
                let mut out = vec![0.0f32; an.band_count() as usize];
                an.levels_db(response, &mut out);
                final_db = out[band as usize];
            }
            center += hop;
        }
        assert!(final_db.is_finite(), "final level should be finite");

        // Re-run to find rise time (first tick within 1 dB of final, measured from the first
        // tone sample entering the tap, i.e. sample index pre_n) and fall time (first tick where
        // the level has dropped ≥ 20 dB from its pre-stop value, measured from the first silent
        // sample, i.e. sample index pre_n + tone_n).
        an.reset();
        center = n;
        let mut rise_time_s: Option<f64> = None;
        let mut level_at_stop = f32::NEG_INFINITY;
        let mut fall_time_s: Option<f64> = None;
        while center <= signal.len() {
            let start = center - n;
            history.copy_from_slice(&signal[start..center]);
            an.process(&history);
            let mut out = vec![0.0f32; an.band_count() as usize];
            an.levels_db(response, &mut out);
            let level = out[band as usize];
            let t_since_tone = (center as f64 - pre_n as f64) / f64::from(FS);
            let t_since_stop = (center as f64 - (pre_n + tone_n) as f64) / f64::from(FS);

            if rise_time_s.is_none() && t_since_tone >= 0.0 && (level - final_db).abs() <= 1.0 {
                rise_time_s = Some(t_since_tone);
            }
            if t_since_stop <= 0.0 {
                level_at_stop = level;
            } else if fall_time_s.is_none() && level <= level_at_stop - 20.0 {
                fall_time_s = Some(t_since_stop);
            }
            center += hop;
        }

        let rise = rise_time_s.expect("never reached within 1 dB of final");
        assert!(
            rise <= rise_budget_s,
            "{response:?} rise time {rise:.3}s exceeds budget {rise_budget_s:.3}s"
        );
        let fall = fall_time_s.expect("never fell 20 dB after stop");
        assert!(
            fall <= fall_budget_s,
            "{response:?} fall time {fall:.3}s exceeds budget {fall_budget_s:.3}s"
        );
    }

    #[test]
    fn ac16_fast_response_time() {
        step_response(Response::Fast, 0.25, 0.40);
    }

    #[test]
    fn ac16_medium_response_time() {
        step_response(Response::Medium, 0.40, 0.85);
    }

    #[test]
    fn ac16_slow_response_time() {
        step_response(Response::Slow, 1.0, 2.6);
    }

    // --- Band table interpolation path (low bands, small N) ---

    #[test]
    fn low_band_uses_interpolation_at_small_fft() {
        // At N = 4096 @ 48 kHz, bin spacing is 11.72 Hz; band 0 (20 Hz, width ~0.6 Hz) has no
        // bin centre inside its edges, so it must use the interpolation path.
        let table = BandTable::new(4096, FS);
        assert!(matches!(table.map[0], BandMap::Interp(..)));
    }

    #[test]
    fn silent_interpolated_band_is_exactly_zero_power() {
        let table = BandTable::new(4096, FS);
        let bins = table.map.len();
        let bin_power = vec![0.0f64; 4096 / 2 + 1];
        let mut out = vec![0.0f64; bins];
        table.reduce(&bin_power, &mut out);
        assert!(out.iter().all(|&p| p == 0.0));
    }
}

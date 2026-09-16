//! Windowed magnitude-spectrum analysis, shared across acceptance tests that need to look at a
//! signal in the frequency domain (H-60 finding: `crates/io/tests/codec_decode.rs` and
//! `crates/dsp/src/dither.rs` each hand-rolled their own DFT for this; H-73 consolidates them
//! here so a future test reaches for one helper instead of writing a third copy).
//!
//! Deliberately **not** built on `realfft`/`rustfft`: ADR-001 §3 rule 3 confines those to `dsp`
//! ("`io` and `engine` resample through `dsp::resample`"), and rule 2 already has `testkit`
//! reimplement its measurements independently of `dsp` "so that tests never grade production
//! code with itself" -- the same reasoning applies to spectral analysis, so this module carries
//! its own small radix-2 FFT (mirroring `vox_module_api::test_util`'s hand-written FFT, kept for
//! the same "no new dependency in a leaf crate" reason).
//!
//! ## Conventions
//! - [`Spectrum::analyze`] takes `samples.len()` as the FFT size directly (no padding); it must
//!   be a power of two.
//! - Magnitude is reported in dB (`20*log10|X[k]|`), **unnormalized**: only levels *within one*
//!   [`Spectrum`] are meaningful to compare (same as the two hand-rolled versions this replaces).
//!   A silent bin reads a large negative number rather than `-inf` (`(|X[k]| + 1e-300).log10()`),
//!   so arithmetic on the result never produces a NaN/inf surprise.
//! - [`Spectrum::dominant_frequency_hz`] and the harmonic/noise-floor helpers all exclude bin 0
//!   (DC); `dominant_frequency_hz` also excludes the Nyquist bin, matching the search range the
//!   two call sites it replaces already used.

use crate::error::TestkitError;

/// An analysis window applied before the FFT. [`Window::BlackmanHarris`] is the default, matching
/// the production analyzer's window (`vox_dsp::analyzer`, `vox_dsp::diagnostics::window`,
/// SPEC-012 §4.3 coefficients): very low spectral leakage, at the cost of a wide main lobe.
/// [`Window::Rectangular`] applies no taper at all -- use it only when a test's tones are
/// deliberately bin-centred (no leakage to control) and neighbouring bins must stay independent
/// (e.g. reading one harmonic bin without smearing energy in from the next).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Window {
    #[default]
    BlackmanHarris,
    Hann,
    Rectangular,
}

/// 4-term Blackman-Harris coefficients (SPEC-012 §4.3), reused by `vox_dsp::analyzer` and
/// `vox_dsp::diagnostics::window` -- same numbers, independently applied here (see module docs).
const BH4: [f64; 4] = [0.35875, 0.48829, 0.14128, 0.01168];

impl Window {
    /// The periodic (DFT-even) window of length `n`.
    fn coefficients(self, n: usize) -> Vec<f64> {
        match self {
            Window::Rectangular => vec![1.0; n],
            Window::Hann => cosine_window(n, &[0.5, 0.5]),
            Window::BlackmanHarris => cosine_window(n, &BH4),
        }
    }
}

/// `w[i] = Σ_k (-1)^k a_k cos(2πki/N)`, periodic (same construction as
/// `vox_dsp::diagnostics::window::WindowKind::coefficients`, independently implemented here).
fn cosine_window(n: usize, terms: &[f64]) -> Vec<f64> {
    let len = n.max(1) as f64;
    (0..n)
        .map(|i| {
            let x = std::f64::consts::TAU * i as f64 / len;
            terms.iter().enumerate().fold(0.0, |acc, (k, &a)| {
                let sign = if k % 2 == 1 { -1.0 } else { 1.0 };
                acc + sign * a * (k as f64 * x).cos()
            })
        })
        .collect()
}

/// In-place radix-2 Cooley-Tukey FFT of `re`/`im` (both length `n`, `n` a power of two).
fn fft_in_place(re: &mut [f64], im: &mut [f64]) {
    let n = re.len();
    debug_assert!(n.is_power_of_two() && im.len() == n);
    // Bit-reversal permutation.
    let bits = n.trailing_zeros();
    for i in 0..n {
        let j = i.reverse_bits() >> (usize::BITS - bits);
        if j > i {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let mut len = 2;
    while len <= n {
        let half = len / 2;
        let step = -2.0 * std::f64::consts::PI / len as f64;
        for k in 0..half {
            let (s, c) = (step * k as f64).sin_cos();
            let mut start = 0;
            while start < n {
                let (a, b) = (start + k, start + k + half);
                let tr = re[b] * c - im[b] * s;
                let ti = re[b] * s + im[b] * c;
                re[b] = re[a] - tr;
                im[b] = im[a] - ti;
                re[a] += tr;
                im[a] += ti;
                start += len;
            }
        }
        len *= 2;
    }
}

/// A windowed magnitude spectrum of one real-valued frame (see module docs for conventions).
#[derive(Debug, Clone)]
pub struct Spectrum {
    /// `magnitude_db[k]` for `k` in `0..=n/2` (DC to Nyquist inclusive).
    magnitude_db: Vec<f64>,
    bin_hz: f64,
}

impl Spectrum {
    /// Analyzes `samples` with `window`. `samples.len()` is the FFT size and must be a power of
    /// two and at least 2.
    ///
    /// # Errors
    /// [`TestkitError::InvalidParameter`] if `samples.len()` isn't a power of two `>= 2`.
    pub fn analyze(
        samples: &[f32],
        sample_rate_hz: u32,
        window: Window,
    ) -> Result<Self, TestkitError> {
        let n = samples.len();
        if n < 2 || !n.is_power_of_two() {
            return Err(TestkitError::InvalidParameter(format!(
                "vox-testkit::spectrum::Spectrum::analyze: samples.len() must be a power of two \
                 >= 2, got {n}"
            )));
        }
        let w = window.coefficients(n);
        let mut re: Vec<f64> = samples
            .iter()
            .zip(&w)
            .map(|(&x, &w)| f64::from(x) * w)
            .collect();
        let mut im = vec![0.0; n];
        fft_in_place(&mut re, &mut im);
        let magnitude_db = re[..=n / 2]
            .iter()
            .zip(&im[..=n / 2])
            .map(|(&re, &im)| 20.0 * (re.hypot(im) + 1e-300).log10())
            .collect();
        Ok(Self {
            magnitude_db,
            bin_hz: f64::from(sample_rate_hz) / n as f64,
        })
    }

    /// Hz per bin (`sample_rate_hz / n`).
    pub fn bin_hz(&self) -> f64 {
        self.bin_hz
    }

    /// Number of bins, `n/2 + 1` (DC to Nyquist inclusive).
    pub fn bin_count(&self) -> usize {
        self.magnitude_db.len()
    }

    /// Magnitude in dB at bin `k` (`0` = DC, `bin_count() - 1` = Nyquist).
    ///
    /// # Panics
    /// If `k >= self.bin_count()`.
    pub fn magnitude_db(&self, k: usize) -> f64 {
        self.magnitude_db[k]
    }

    /// The bin nearest `freq_hz` (clamped to `0..bin_count()`).
    fn bin_for_hz(&self, freq_hz: f64) -> usize {
        let bin = (freq_hz / self.bin_hz).round();
        if bin <= 0.0 {
            0
        } else if bin >= (self.bin_count() - 1) as f64 {
            self.bin_count() - 1
        } else {
            bin as usize
        }
    }

    /// Magnitude in dB at the bin nearest `freq_hz`.
    pub fn level_at_hz(&self, freq_hz: f64) -> f64 {
        self.magnitude_db[self.bin_for_hz(freq_hz)]
    }

    /// The frequency (Hz) of the highest-magnitude bin, excluding DC and the Nyquist bin.
    ///
    /// # Panics
    /// If `bin_count() < 3` (nothing left to search once DC and Nyquist are excluded).
    pub fn dominant_frequency_hz(&self) -> f64 {
        assert!(
            self.bin_count() >= 3,
            "vox-testkit::spectrum::Spectrum::dominant_frequency_hz: too few bins ({}) once DC \
             and Nyquist are excluded",
            self.bin_count()
        );
        let (bin, _) = self.magnitude_db[1..self.bin_count() - 1]
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.total_cmp(b.1))
            .expect("non-empty by the assert above");
        (bin + 1) as f64 * self.bin_hz
    }

    /// Level of each of `harmonic_numbers` (`2` = 2nd harmonic, `3` = 3rd, ...) relative to the
    /// fundamental's own level, in dB (negative = quieter than the fundamental, i.e. dBc).
    pub fn harmonic_levels_db(&self, fundamental_hz: f64, harmonic_numbers: &[u32]) -> Vec<f64> {
        let f0_level_db = self.level_at_hz(fundamental_hz);
        harmonic_numbers
            .iter()
            .map(|&k| self.level_at_hz(fundamental_hz * f64::from(k)) - f0_level_db)
            .collect()
    }

    /// Median magnitude (dB) of the bins within `window_hz` of `center_hz`, excluding the
    /// `exclude_hz`-wide band immediately around it -- a local noise-floor / "median of the
    /// neighbouring bins" reference level (SPEC-005 AC-4's own methodology).
    pub fn local_noise_floor_db(&self, center_hz: f64, exclude_hz: f64, window_hz: f64) -> f64 {
        let center = self.bin_for_hz(center_hz);
        let exclude_bins = (exclude_hz / self.bin_hz).round().max(0.0) as usize;
        let window_bins = (window_hz / self.bin_hz).round().max(0.0) as usize;
        let lo = center.saturating_sub(window_bins);
        let hi = (center + window_bins).min(self.bin_count() - 1);
        let mut levels: Vec<f64> = (lo..=hi)
            .filter(|&k| k.abs_diff(center) > exclude_bins)
            .map(|k| self.magnitude_db[k])
            .collect();
        assert!(
            !levels.is_empty(),
            "vox-testkit::spectrum::Spectrum::local_noise_floor_db: exclude_hz ({exclude_hz}) \
             leaves no bins inside window_hz ({window_hz})"
        );
        levels.sort_by(f64::total_cmp);
        levels[levels.len() / 2]
    }

    /// Global noise-floor estimate (dB): the median magnitude across every bin except DC --
    /// useful when there is no single tone to steer clear of (e.g. a white-noise floor).
    pub fn noise_floor_db(&self) -> f64 {
        let mut levels: Vec<f64> = self.magnitude_db[1..].to_vec();
        levels.sort_by(f64::total_cmp);
        levels[levels.len() / 2]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE_HZ: u32 = 48_000;
    const N: usize = 8192;

    fn bin_hz() -> f64 {
        f64::from(RATE_HZ) / N as f64
    }

    fn sine(freq_hz: f64, amp: f64) -> Vec<f32> {
        (0..N)
            .map(|i| {
                (amp * (2.0 * std::f64::consts::PI * freq_hz * i as f64 / f64::from(RATE_HZ)).sin())
                    as f32
            })
            .collect()
    }

    #[test]
    fn fft_matches_naive_dft() {
        // Verifies the hand-rolled radix-2 FFT against a textbook O(n^2) DFT on a small,
        // non-trivial (not a pure sine) signal -- same pattern as
        // `vox_module_api::test_util::fft`'s `matches_naive_dft`, independently written here.
        let n = 64;
        let x: Vec<f64> = (0..n).map(|i| ((i * 7 % 13) as f64 - 6.0) * 0.1).collect();
        let (mut re, mut im) = (x.clone(), vec![0.0; n]);
        fft_in_place(&mut re, &mut im);
        for k in 0..n {
            let (mut sr, mut si) = (0.0, 0.0);
            for (i, &v) in x.iter().enumerate() {
                let a = -2.0 * std::f64::consts::PI * (k * i) as f64 / n as f64;
                sr += v * a.cos();
                si += v * a.sin();
            }
            assert!((re[k] - sr).abs() < 1e-9, "re[{k}]: {} vs {sr}", re[k]);
            assert!((im[k] - si).abs() < 1e-9, "im[{k}]: {} vs {si}", im[k]);
        }
    }

    #[test]
    fn analyze_rejects_non_power_of_two_length() {
        assert!(Spectrum::analyze(&[0.0; 100], RATE_HZ, Window::Rectangular).is_err());
        assert!(Spectrum::analyze(&[], RATE_HZ, Window::Rectangular).is_err());
    }

    #[test]
    fn dominant_frequency_hz_finds_a_bin_centred_tone_under_every_window() {
        // Bin 171 of 8192 @ 48 kHz = 1001.953125 Hz exactly (dyadic: 375/64 * 171).
        let k0: i32 = 171;
        let f0_hz = f64::from(k0) * bin_hz();
        let samples = sine(f0_hz, 0.5);
        for window in [Window::Rectangular, Window::Hann, Window::BlackmanHarris] {
            let spectrum = Spectrum::analyze(&samples, RATE_HZ, window).unwrap();
            let found = spectrum.dominant_frequency_hz();
            assert!(
                (found - f0_hz).abs() < 1e-6,
                "{window:?}: found {found} Hz, expected exactly {f0_hz} Hz (bin-centred)"
            );
        }
    }

    #[test]
    fn dominant_frequency_hz_finds_an_off_bin_tone_within_half_a_bin() {
        // 997 Hz doesn't land on a bin at this size/rate (bin_hz ~= 5.86 Hz): the found peak
        // should still be the closest bin, under every window.
        let f0_hz = 997.0;
        let samples = sine(f0_hz, 0.5);
        for window in [Window::Rectangular, Window::Hann, Window::BlackmanHarris] {
            let spectrum = Spectrum::analyze(&samples, RATE_HZ, window).unwrap();
            let found = spectrum.dominant_frequency_hz();
            assert!(
                (found - f0_hz).abs() <= bin_hz() / 2.0 + 1e-9,
                "{window:?}: found {found} Hz, expected within {} Hz of {f0_hz} Hz",
                bin_hz() / 2.0
            );
        }
    }

    #[test]
    fn level_at_hz_reads_a_full_scale_bin_centred_tone_near_zero_db_under_rectangular() {
        // Unnormalized rectangular-window FFT of a full-amplitude bin-centred sine: peak bin
        // magnitude is `n/2` linear, i.e. `20*log10(n/2)` dB.
        let k0: i32 = 171;
        let f0_hz = f64::from(k0) * bin_hz();
        let samples = sine(f0_hz, 1.0);
        let spectrum = Spectrum::analyze(&samples, RATE_HZ, Window::Rectangular).unwrap();
        let expected_db = 20.0 * (N as f64 / 2.0).log10();
        let got_db = spectrum.level_at_hz(f0_hz);
        assert!(
            (got_db - expected_db).abs() < 0.01,
            "got {got_db} dB, expected {expected_db} dB"
        );
    }

    #[test]
    fn harmonic_levels_db_reads_a_known_harmonic_series() {
        // Fundamental at bin 171, 2nd/3rd harmonics at exact bins too (2*171, 3*171), each a
        // known number of dB below the fundamental -- built directly from linear amplitudes so
        // the expected dB gaps are exact.
        let k0: i32 = 171;
        let f0_hz = f64::from(k0) * bin_hz();
        let amp0 = 0.5;
        let amp2 = amp0 * 10f64.powf(-20.0 / 20.0); // -20 dB
        let amp3 = amp0 * 10f64.powf(-26.0 / 20.0); // -26 dB
        let samples: Vec<f32> = (0..N)
            .map(|i| {
                let t = i as f64 / f64::from(RATE_HZ);
                let tau = 2.0 * std::f64::consts::PI;
                (amp0 * (tau * f0_hz * t).sin()
                    + amp2 * (tau * 2.0 * f0_hz * t).sin()
                    + amp3 * (tau * 3.0 * f0_hz * t).sin()) as f32
            })
            .collect();
        let spectrum = Spectrum::analyze(&samples, RATE_HZ, Window::Rectangular).unwrap();
        let levels = spectrum.harmonic_levels_db(f0_hz, &[2, 3]);
        assert!(
            (levels[0] - (-20.0)).abs() < 0.05,
            "2nd harmonic: {} dB, expected -20 dB",
            levels[0]
        );
        assert!(
            (levels[1] - (-26.0)).abs() < 0.05,
            "3rd harmonic: {} dB, expected -26 dB",
            levels[1]
        );
    }

    #[test]
    fn noise_floor_db_of_white_noise_is_stable_across_the_spectrum() {
        // A dithered white-noise-ish signal (no tone): the global median and a handful of local
        // medians (well away from DC/Nyquist) should all land within a couple dB of each other.
        let mut rng = crate::prng::Pcg32::new(7, 0);
        let samples: Vec<f32> = (0..N).map(|_| (rng.next_signed() * 0.2) as f32).collect();
        let spectrum = Spectrum::analyze(&samples, RATE_HZ, Window::Rectangular).unwrap();
        let global = spectrum.noise_floor_db();
        for center_hz in [2_000.0, 6_000.0, 12_000.0, 18_000.0] {
            let local = spectrum.local_noise_floor_db(center_hz, 0.0, 50.0 * bin_hz());
            assert!(
                (local - global).abs() < 3.0,
                "local noise floor near {center_hz} Hz ({local} dB) vs global ({global} dB) \
                 differ by more than 3 dB"
            );
        }
    }

    #[test]
    fn local_noise_floor_db_excludes_the_centre_band() {
        // A strong bin-centred tone plus a flat noise floor: the local noise floor at the tone's
        // own frequency must read near the noise floor, not the tone -- proof the exclusion band
        // actually works.
        let k0: i32 = 171;
        let f0_hz = f64::from(k0) * bin_hz();
        let mut rng = crate::prng::Pcg32::new(11, 0);
        let tone = sine(f0_hz, 0.5);
        let samples: Vec<f32> = tone
            .iter()
            .map(|&x| x + (rng.next_signed() * 0.01) as f32)
            .collect();
        let spectrum = Spectrum::analyze(&samples, RATE_HZ, Window::Rectangular).unwrap();
        let tone_level = spectrum.level_at_hz(f0_hz);
        let floor = spectrum.local_noise_floor_db(f0_hz, 3.0 * bin_hz(), 40.0 * bin_hz());
        assert!(
            tone_level - floor > 20.0,
            "tone level {tone_level} dB should be well above the local floor {floor} dB"
        );
    }
}

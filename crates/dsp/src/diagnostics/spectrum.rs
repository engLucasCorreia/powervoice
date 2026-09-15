//! Configurable power spectra (H-42, SPEC-007 §8.3–§8.4): one windowed real FFT per frame with
//! the same **sine-normalized** convention as the live analyzer (`P[k] = |X[k]|² / (Σw/2)²`, so
//! a full-scale sine centred on a bin reads 0 dB whatever the window), and the Welch-style
//! long-term average ([`Ltas`]: 50 % overlapped frames, mean power per bin).
//!
//! Band power (SPEC-007 §8.4) is `Σ P[k] / ENBW` over the bins whose centre lies in the band:
//! a sine of amplitude `A` reads `A²` (0 dB at full scale), a noise band of RMS `σ` reads `2σ²`
//! (its RMS dBFS + 3.01 dB, the AES17 sine-referenced scale). Ratios of band powers — every
//! diagnostic in [`super::features`] — are therefore window- and FFT-size-independent.
//!
//! Off the audio thread only (the engine's analyzer publisher, the LTAS job thread).

use std::sync::Arc;

use realfft::num_complex::Complex;
use realfft::{RealFftPlanner, RealToComplex};

use super::window::{WindowKind, enbw_bins};

/// Smallest FFT size the Spectrum Inspector / LTAS job offer (SPEC-007 §8.3).
pub const MIN_FFT_SIZE: u32 = 1024;
/// Largest FFT size the Spectrum Inspector / LTAS job offer (SPEC-007 §8.3).
pub const MAX_FFT_SIZE: u32 = 32_768;
/// The LTAS job's and the Inspector's default FFT size (SPEC-007 §8.3).
pub const DEFAULT_FFT_SIZE: u32 = 16_384;

/// A power of two in [`MIN_FFT_SIZE`] ..= [`MAX_FFT_SIZE`].
pub fn is_valid_fft_size(fft_size: u32) -> bool {
    fft_size.is_power_of_two() && (MIN_FFT_SIZE..=MAX_FFT_SIZE).contains(&fft_size)
}

/// One windowed real FFT of `fft_size` points producing sine-normalized bin powers. Allocation-
/// free after construction.
pub struct PowerSpectrum {
    fft: Arc<dyn RealToComplex<f32>>,
    kind: WindowKind,
    window: Box<[f32]>,
    frame: Vec<f32>,
    spectrum: Vec<Complex<f32>>,
    scratch: Vec<Complex<f32>>,
    /// `1 / (Σw/2)²`.
    inv_norm_sq: f64,
    enbw_bins: f64,
}

impl PowerSpectrum {
    /// # Panics
    /// If `fft_size` is not a power of two ≥ 4.
    pub fn new(fft_size: usize, kind: WindowKind) -> Self {
        assert!(
            fft_size.is_power_of_two() && fft_size >= 4,
            "FFT size must be a power of two ≥ 4"
        );
        let fft = RealFftPlanner::<f32>::new().plan_fft_forward(fft_size);
        let window = kind.coefficients(fft_size);
        let half_sum = window.iter().sum::<f64>() / 2.0;
        Self {
            scratch: vec![Complex::default(); fft.get_scratch_len()],
            frame: fft.make_input_vec(),
            spectrum: fft.make_output_vec(),
            enbw_bins: enbw_bins(&window),
            window: window.iter().map(|&w| w as f32).collect(),
            inv_norm_sq: 1.0 / (half_sum * half_sum),
            kind,
            fft,
        }
    }

    pub fn fft_size(&self) -> usize {
        self.window.len()
    }

    /// `fft_size / 2 + 1`.
    pub fn bins(&self) -> usize {
        self.spectrum.len()
    }

    pub fn window(&self) -> WindowKind {
        self.kind
    }

    /// Equivalent noise bandwidth of the window, in bins.
    pub fn enbw_bins(&self) -> f64 {
        self.enbw_bins
    }

    /// Sine-normalized bin powers of `input` (length `fft_size`) into `out` (length `bins()`).
    /// Non-finite samples count as 0 (SPEC-007 AC-3 convention).
    pub fn power(&mut self, input: &[f32], out: &mut [f64]) {
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

/// Long-term average spectrum (Welch): `fft_size`-point frames at a hop of `fft_size / 2`, the
/// **mean** sine-normalized power per bin over every frame (SPEC-007 §8.5). A stream shorter
/// than one frame is analysed as one zero-padded frame, so any non-empty selection has a
/// spectrum.
pub struct Ltas {
    spectrum: PowerSpectrum,
    hop: usize,
    pending: Vec<f32>,
    frame_power: Vec<f64>,
    acc: Vec<f64>,
    frames: u64,
}

impl Ltas {
    /// # Panics
    /// If `fft_size` is not a power of two ≥ 4.
    pub fn new(fft_size: usize, kind: WindowKind) -> Self {
        let spectrum = PowerSpectrum::new(fft_size, kind);
        let bins = spectrum.bins();
        Self {
            hop: fft_size / 2,
            pending: Vec::with_capacity(fft_size * 2),
            frame_power: vec![0.0; bins],
            acc: vec![0.0; bins],
            frames: 0,
            spectrum,
        }
    }

    pub fn fft_size(&self) -> usize {
        self.spectrum.fft_size()
    }

    pub fn window(&self) -> WindowKind {
        self.spectrum.window()
    }

    pub fn enbw_bins(&self) -> f64 {
        self.spectrum.enbw_bins()
    }

    /// Frames averaged so far.
    pub fn frames(&self) -> u64 {
        self.frames
    }

    /// Feeds more samples; every complete frame is analysed and added to the average.
    pub fn push(&mut self, samples: &[f32]) {
        let n = self.spectrum.fft_size();
        let mut rest = samples;
        while !rest.is_empty() {
            let want = n - self.pending.len().min(n);
            let take = want.min(rest.len());
            self.pending.extend_from_slice(&rest[..take]);
            rest = &rest[take..];
            if self.pending.len() >= n {
                self.analyse_frame();
                self.pending.drain(..self.hop);
            }
        }
    }

    fn analyse_frame(&mut self) {
        let n = self.spectrum.fft_size();
        self.spectrum
            .power(&self.pending[..n], &mut self.frame_power);
        for (a, &p) in self.acc.iter_mut().zip(&self.frame_power) {
            *a += p;
        }
        self.frames += 1;
    }

    /// The mean power per bin (`fft_size / 2 + 1` values). Consumes any partial frame first
    /// (zero-padded) when no full frame was ever seen; empty when nothing was pushed at all.
    pub fn finish(mut self) -> Vec<f64> {
        if self.frames == 0 {
            if self.pending.is_empty() {
                return vec![0.0; self.acc.len()];
            }
            self.pending.resize(self.spectrum.fft_size(), 0.0);
            self.analyse_frame();
        }
        let inv = 1.0 / self.frames as f64;
        self.acc.iter().map(|&a| a * inv).collect()
    }
}

/// Power → dB (`0 → −∞`, never NaN).
pub fn power_db(p: f64) -> f64 {
    if p > 0.0 {
        10.0 * p.log10()
    } else {
        f64::NEG_INFINITY
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FS: u32 = 48_000;

    fn sine(freq_hz: f64, amplitude: f64, len: usize) -> Vec<f32> {
        let w = std::f64::consts::TAU * freq_hz / f64::from(FS);
        (0..len)
            .map(|i| (amplitude * (w * i as f64).sin()) as f32)
            .collect()
    }

    fn bin_hz(fft: usize) -> f64 {
        f64::from(FS) / fft as f64
    }

    #[test]
    fn a_bin_centred_sine_reads_its_level_with_every_window() {
        let fft = 8192;
        let freq = 171.0 * bin_hz(fft); // exactly on bin 171
        let x = sine(freq, 0.5, fft);
        for kind in WindowKind::ALL {
            let mut s = PowerSpectrum::new(fft, kind);
            let mut p = vec![0.0; s.bins()];
            s.power(&x, &mut p);
            let db = power_db(p[171]);
            assert!(
                (db - (-6.0206)).abs() < 0.02,
                "{kind:?}: {db} dB instead of −6.02"
            );
        }
    }

    #[test]
    fn band_power_of_a_sine_is_its_amplitude_squared() {
        // Σ P / ENBW over the main lobe = A² whatever the window and the bin offset.
        let fft = 8192;
        let x = sine(1234.5, 0.25, fft);
        for kind in [
            WindowKind::Hann,
            WindowKind::BlackmanHarris,
            WindowKind::FlatTop,
        ] {
            let mut s = PowerSpectrum::new(fft, kind);
            let mut p = vec![0.0; s.bins()];
            s.power(&x, &mut p);
            let lo = (1100.0 / bin_hz(fft)) as usize;
            let hi = (1400.0 / bin_hz(fft)) as usize;
            let band: f64 = p[lo..hi].iter().sum::<f64>() / s.enbw_bins();
            assert!(
                (power_db(band) - power_db(0.0625)).abs() < 0.05,
                "{kind:?}: band {} dB",
                power_db(band)
            );
        }
    }

    #[test]
    fn white_noise_ltas_sits_at_sigma_plus_the_window_offset() {
        // SPEC-007 §4.8.5 generalised: per-bin level = σ_dB + 10·log10(4·ENBW/N).
        let fft = 8192;
        let noise = vox_testkit::signal::white_noise(7, -20.0, 20.0, FS).unwrap();
        for kind in [WindowKind::Hann, WindowKind::BlackmanHarris] {
            let mut ltas = Ltas::new(fft, kind);
            let enbw = ltas.enbw_bins();
            ltas.push(&noise);
            assert!(ltas.frames() > 200);
            let mean = ltas.finish();
            let avg: f64 = mean[10..mean.len() - 10].iter().sum::<f64>() / (mean.len() - 20) as f64;
            let expected = -20.0 + 10.0 * (4.0 * enbw / fft as f64).log10();
            assert!(
                (power_db(avg) - expected).abs() < 0.2,
                "{kind:?}: {} dB vs {expected}",
                power_db(avg)
            );
        }
    }

    #[test]
    fn ltas_equals_the_average_of_its_frames_on_a_known_signal() {
        // First second: a −6 dBFS sine at 1 kHz; second: the same at 3 kHz. The long-term
        // average holds each at half its power (−3.01 dB), give or take the one frame that
        // straddles the switch.
        let fft = 4096;
        let f1 = 85.0 * bin_hz(fft);
        let f2 = 256.0 * bin_hz(fft);
        let mut x = sine(f1, 0.5, FS as usize);
        x.extend(sine(f2, 0.5, FS as usize));
        let mut ltas = Ltas::new(fft, WindowKind::Hann);
        // Arbitrary push sizes must not change the answer.
        for chunk in x.chunks(777) {
            ltas.push(chunk);
        }
        let frames = ltas.frames();
        let mean = ltas.finish();

        // Reference: the same frames computed one by one.
        let mut s = PowerSpectrum::new(fft, WindowKind::Hann);
        let mut p = vec![0.0; s.bins()];
        let mut acc = vec![0.0; s.bins()];
        let mut count = 0u64;
        let mut start = 0;
        while start + fft <= x.len() {
            s.power(&x[start..start + fft], &mut p);
            for (a, v) in acc.iter_mut().zip(&p) {
                *a += v;
            }
            count += 1;
            start += fft / 2;
        }
        assert_eq!(frames, count);
        for (k, (&m, &a)) in mean.iter().zip(&acc).enumerate() {
            let r = a / count as f64;
            assert!((m - r).abs() <= 1e-12 * r.abs().max(1e-30), "bin {k}");
        }
        for bin in [85, 256] {
            assert!(
                (power_db(mean[bin]) - (-6.0206 - 3.0103)).abs() < 0.3,
                "bin {bin}: {} dB",
                power_db(mean[bin])
            );
        }
    }

    #[test]
    fn a_short_stream_is_one_zero_padded_frame() {
        let mut ltas = Ltas::new(1024, WindowKind::Hann);
        ltas.push(&sine(1000.0, 0.5, 300));
        let mean = ltas.finish();
        assert_eq!(mean.len(), 513);
        assert!(mean.iter().any(|&p| p > 0.0));
        assert!(
            Ltas::new(1024, WindowKind::Hann)
                .finish()
                .iter()
                .all(|&p| p == 0.0)
        );
    }

    #[test]
    fn fft_size_validation() {
        assert!(is_valid_fft_size(1024));
        assert!(is_valid_fft_size(32_768));
        assert!(!is_valid_fft_size(512));
        assert!(!is_valid_fft_size(65_536));
        assert!(!is_valid_fft_size(3000));
    }

    #[test]
    fn non_finite_samples_count_as_zero() {
        let mut s = PowerSpectrum::new(1024, WindowKind::Hann);
        let mut p = vec![0.0; s.bins()];
        let mut x = vec![0.0f32; 1024];
        x[10] = f32::NAN;
        x[20] = f32::INFINITY;
        s.power(&x, &mut p);
        assert!(p.iter().all(|&v| v == 0.0));
    }
}

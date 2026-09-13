//! Spectrogram frame analysis (SPEC-007 §4.2–§4.4, ADR-003 `VXST`): Hann-windowed real FFT
//! frames, sine-normalized levels, the overview mean-power frames, the u8 quantizer and the
//! zoom → hop rule. Off the audio thread only (the engine's tile workers call this); pure and
//! deterministic — the same input always gives the same codes on the same machine.
//!
//! Conventions (normative, SPEC-007 §4.2):
//! - A frame centred at `c` reads `x[c − N/2 … c + N/2)`, zeros outside the document. Periodic
//!   Hann `w[n] = 0.5 − 0.5·cos(2πn/N)`, so `Σw = N/2`.
//! - Bin power `P[k] = |X[k]|² / (Σw/2)²`, level `10·log10(P)`: a full-scale sine centred on a
//!   bin reads 0 dB.
//! - Overview frames (`hop > N`): the mean power of `2·hop/N` sub-frames at stride `N/2`
//!   (COLA for Hann), centred at `c − hop/2 + (j + ½)·N/2` (§4.4). A **preview** frame is the
//!   single frame at `c`.
//! - Quantization: `v = round(255 · clamp((L + 150)/156, 0, 1))`; −∞ and NaN → 0.
//! - Non-finite input samples are analysed as 0.0, so no code is ever derived from them
//!   (SPEC-007 AC-3).

use std::sync::Arc;

use realfft::num_complex::Complex;
use realfft::{RealFftPlanner, RealToComplex};

/// Quantization floor/ceiling of a tile code (ADR-003 `VXST` `q_floor_db`/`q_ceil_db`).
pub const Q_FLOOR_DB: f32 = -150.0;
pub const Q_CEIL_DB: f32 = 6.0;
/// Smallest and largest FFT sizes the spectrogram offers (SPEC-007 §2.6).
pub const MIN_FFT_SIZE: u32 = 256;
pub const MAX_FFT_SIZE: u32 = 16_384;
/// The hop is never smaller than `fft_size / MIN_HOP_DIVISOR` (93.75 % overlap, SPEC-007 §4.3).
pub const MIN_HOP_DIVISOR: u32 = 16;

/// `true` for the FFT sizes SPEC-007 §2.6 offers: powers of two in 256 ..= 16 384.
pub fn is_valid_fft_size(fft_size: u32) -> bool {
    fft_size.is_power_of_two() && (MIN_FFT_SIZE..=MAX_FFT_SIZE).contains(&fft_size)
}

/// `true` for a hop the tile grid accepts at `fft_size`: a power of two ≥ `fft_size / 16`
/// (SPEC-007 §3 `hop`).
pub fn is_valid_hop(fft_size: u32, hop: u32) -> bool {
    hop.is_power_of_two() && hop >= fft_size / MIN_HOP_DIVISOR
}

/// SPEC-007 §2.6 "Auto": the power of two nearest to `2048 · fs / 48 000`, clamped to
/// 256 ..= 16 384 (2048 at 44.1/48 kHz, 4096 at 88.2/96 kHz, 1024 at 22.05/24 kHz, 256 at 8 kHz).
pub fn auto_fft_size(sample_rate_hz: u32) -> u32 {
    let target = 2048.0 * f64::from(sample_rate_hz) / 48_000.0;
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

/// SPEC-007 §4.3: `hop = max(pow2_floor(spp_dev), N/16)`, where `spp_dev` is document samples per
/// **device** pixel. `spp_dev < 1` (or non-finite) counts as 1.
pub fn hop_for_zoom(spp_dev: f64, fft_size: u32) -> u32 {
    let spp = if spp_dev.is_finite() && spp_dev >= 1.0 {
        spp_dev.min(f64::from(1u32 << 31))
    } else {
        1.0
    };
    // Largest power of two ≤ spp (spp ≥ 1, so the shift is in 0..=31).
    let floor = 1u32 << (spp as u32).ilog2();
    floor.max(fft_size / MIN_HOP_DIVISOR)
}

/// `true` when frames at `hop` are overview frames (mean power of sub-frames, SPEC-007 §4.4).
pub fn is_overview(fft_size: u32, hop: u32) -> bool {
    hop > fft_size
}

/// What one frame of a tile analyses: the input window relative to the frame centre and the
/// number of `N`-sample sub-frames (stride `N/2`) whose powers are averaged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameInput {
    /// Offset of the first input sample from the frame centre (negative).
    pub offset: i64,
    /// Input length in samples.
    pub len: usize,
    /// Sub-frames averaged (1 for plain and preview frames).
    pub sub_frames: usize,
}

/// The input a frame reads (SPEC-007 §4.2, §4.4): `[c − N/2, c + N/2)` for plain frames
/// (`hop ≤ N`) and previews; `[c − hop/2 − N/4, c + hop/2 + N/4)` with `2·hop/N` sub-frames for
/// refined overview frames.
pub fn frame_input(fft_size: u32, hop: u32, preview: bool) -> FrameInput {
    let n = fft_size as usize;
    if is_overview(fft_size, hop) && !preview {
        let hop = hop as usize;
        FrameInput {
            offset: -((hop / 2 + n / 4) as i64),
            len: hop + n / 2,
            sub_frames: 2 * hop / n,
        }
    } else {
        FrameInput {
            offset: -((n / 2) as i64),
            len: n,
            sub_frames: 1,
        }
    }
}

/// Level in dB → tile code (ADR-003 quantization). −∞ and NaN → 0; ≥ +6 dB → 255.
pub fn quantize_db(level_db: f64) -> u8 {
    if level_db.is_nan() {
        return 0;
    }
    let t =
        ((level_db - f64::from(Q_FLOOR_DB)) / f64::from(Q_CEIL_DB - Q_FLOOR_DB)).clamp(0.0, 1.0);
    (255.0 * t).round() as u8
}

/// Normalized bin power → tile code (`10·log10(p)`, then [`quantize_db`]); `p = 0` → 0.
pub fn quantize_power(power: f64) -> u8 {
    if power > 0.0 {
        quantize_db(10.0 * power.log10())
    } else {
        0
    }
}

/// Tile code → dB (`−150 + v·156/255`, 0.6118 dB step), as the UI dequantizes it.
pub fn dequantize(code: u8) -> f32 {
    Q_FLOOR_DB + f32::from(code) * (Q_CEIL_DB - Q_FLOOR_DB) / 255.0
}

/// One Hann-windowed real FFT of size `N` with scratch buffers, producing sine-normalized bin
/// powers. Reused across frames; allocation-free after construction.
pub struct Stft {
    fft: Arc<dyn RealToComplex<f32>>,
    window: Box<[f32]>,
    frame: Vec<f32>,
    spectrum: Vec<Complex<f32>>,
    scratch: Vec<Complex<f32>>,
    /// `1 / (Σw/2)²`.
    inv_norm_sq: f64,
}

impl Stft {
    /// An STFT of `fft_size` points (periodic Hann).
    ///
    /// # Panics
    /// If `fft_size` is not a power of two ≥ 4.
    pub fn new(fft_size: usize) -> Self {
        assert!(
            fft_size.is_power_of_two() && fft_size >= 4,
            "FFT size must be a power of two ≥ 4"
        );
        let fft = RealFftPlanner::<f32>::new().plan_fft_forward(fft_size);
        let window_f64: Vec<f64> = (0..fft_size)
            .map(|i| 0.5 - 0.5 * (std::f64::consts::TAU * i as f64 / fft_size as f64).cos())
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

    /// FFT size `N`.
    pub fn fft_size(&self) -> usize {
        self.window.len()
    }

    /// Bin count `N/2 + 1` (bin 0 = DC).
    pub fn bins(&self) -> usize {
        self.spectrum.len()
    }

    /// Adds the normalized bin powers `|X[k]|²/(Σw/2)²` of one frame to `acc`.
    /// `input.len()` must be `N` and `acc.len()` must be `N/2 + 1`; non-finite samples count as 0.
    pub fn accumulate_power(&mut self, input: &[f32], acc: &mut [f64]) {
        debug_assert_eq!(input.len(), self.window.len());
        debug_assert_eq!(acc.len(), self.spectrum.len());
        for ((dst, &x), &w) in self.frame.iter_mut().zip(input).zip(self.window.iter()) {
            *dst = if x.is_finite() { x * w } else { 0.0 };
        }
        // Lengths match the plan by construction, so this cannot fail.
        let ok = self
            .fft
            .process_with_scratch(&mut self.frame, &mut self.spectrum, &mut self.scratch)
            .is_ok();
        debug_assert!(ok);
        for (a, c) in acc.iter_mut().zip(self.spectrum.iter()) {
            let re = f64::from(c.re);
            let im = f64::from(c.im);
            *a += (re * re + im * im) * self.inv_norm_sq;
        }
    }
}

/// Turns one frame's input into its `N/2 + 1` tile codes: the mean normalized power of its
/// sub-frames ([`FrameInput`]), quantized. Reused across frames.
pub struct FrameAnalyzer {
    stft: Stft,
    acc: Vec<f64>,
}

impl FrameAnalyzer {
    /// An analyzer for `fft_size`-point frames. See [`Stft::new`] for the panics.
    pub fn new(fft_size: usize) -> Self {
        let stft = Stft::new(fft_size);
        let acc = vec![0.0; stft.bins()];
        Self { stft, acc }
    }

    /// FFT size `N`.
    pub fn fft_size(&self) -> usize {
        self.stft.fft_size()
    }

    /// Bin count `N/2 + 1`.
    pub fn bins(&self) -> usize {
        self.stft.bins()
    }

    /// Mean normalized bin powers of `sub_frames` sub-frames of `input` (sub-frame `j` =
    /// `input[j·N/2 .. j·N/2 + N]`), into the internal accumulator; returns it.
    /// `input.len()` must be at least `(sub_frames − 1)·N/2 + N`.
    pub fn mean_power(&mut self, input: &[f32], sub_frames: usize) -> &[f64] {
        let n = self.stft.fft_size();
        self.acc.fill(0.0);
        for j in 0..sub_frames {
            let start = j * n / 2;
            self.stft
                .accumulate_power(&input[start..start + n], &mut self.acc);
        }
        let inv = 1.0 / sub_frames.max(1) as f64;
        for a in &mut self.acc {
            *a *= inv;
        }
        &self.acc
    }

    /// The frame's tile codes (`out.len() == N/2 + 1`) from its input ([`frame_input`]).
    pub fn frame_codes(&mut self, input: &[f32], sub_frames: usize, out: &mut [u8]) {
        let powers = self.mean_power(input, sub_frames);
        for (o, &p) in out.iter_mut().zip(powers) {
            *o = quantize_power(p);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FS: u32 = 48_000;

    fn sine(freq_hz: f64, level_dbfs: f64, len: usize) -> Vec<f32> {
        let a = 10f64.powf(level_dbfs / 20.0);
        let w = std::f64::consts::TAU * freq_hz / f64::from(FS);
        (0..len)
            .map(|i| (a * (w * i as f64).sin()) as f32)
            .collect()
    }

    /// Codes of the frame centred at `center` of `signal` (zeros outside), like a tile does.
    fn frame_at(signal: &[f32], center: i64, fft: u32, hop: u32, preview: bool) -> Vec<u8> {
        let fi = frame_input(fft, hop, preview);
        let start = center + fi.offset;
        let input: Vec<f32> = (0..fi.len as i64)
            .map(|i| {
                let p = start + i;
                if p >= 0 && (p as usize) < signal.len() {
                    signal[p as usize]
                } else {
                    0.0
                }
            })
            .collect();
        let mut an = FrameAnalyzer::new(fft as usize);
        let mut out = vec![0u8; an.bins()];
        an.frame_codes(&input, fi.sub_frames, &mut out);
        out
    }

    fn peak_bin(codes: &[u8]) -> usize {
        codes
            .iter()
            .enumerate()
            .max_by_key(|&(_, c)| *c)
            .map(|(i, _)| i)
            .unwrap()
    }

    #[test]
    fn quantizer_matches_adr_003_and_its_extremes() {
        assert_eq!(quantize_db(-150.0), 0);
        assert_eq!(quantize_db(6.0), 255);
        assert_eq!(quantize_db(f64::NEG_INFINITY), 0);
        assert_eq!(quantize_db(f64::NAN), 0);
        assert_eq!(quantize_db(10.0), 255, "+10 dB clips to code 255");
        assert_eq!(quantize_db(-400.0), 0);
        assert_eq!(quantize_power(0.0), 0);
        // Round trip within half a step (0.306 dB) everywhere in range.
        for tenth in -1500..=60 {
            let db = f64::from(tenth) / 10.0;
            let back = f64::from(dequantize(quantize_db(db)));
            assert!((back - db).abs() <= 0.306, "{db} → {back}");
        }
        assert!((dequantize(255) - 6.0).abs() < 1e-5);
        assert!((dequantize(0) + 150.0).abs() < 1e-5);
    }

    /// SPEC-007 AC-1: bin-centred 1 007.8125 Hz (bin 43) and 1 000 Hz (δ = 1/3) at −20 dBFS, FFT
    /// 2048, every interior frame.
    #[test]
    fn ac1_tone_level_and_peak_bin_at_fft_2048() {
        let signal = sine(1007.8125, -20.0, 10 * FS as usize);
        for center in (4096..signal.len() as i64 - 4096).step_by(37_111) {
            let codes = frame_at(&signal, center, 2048, 128, false);
            assert_eq!(peak_bin(&codes), 43);
            let level = dequantize(codes[43]);
            assert!((level + 20.0).abs() <= 0.35, "bin-centred level {level}");
        }
        let signal = sine(1000.0, -20.0, 10 * FS as usize);
        for center in (4096..signal.len() as i64 - 4096).step_by(37_111) {
            let codes = frame_at(&signal, center, 2048, 128, false);
            assert_eq!(peak_bin(&codes), 43);
            let level = dequantize(codes[43]);
            assert!((level + 20.63).abs() <= 0.35, "1 kHz level {level}");
        }
    }

    /// SPEC-007 AC-1: every FFT size reads a bin-centred −20 dBFS tone at −20.00 ± 0.35 dB, and a
    /// full-scale bin-centred sine at 0.00 ± 0.35 dB.
    #[test]
    fn ac1_every_fft_size_and_full_scale() {
        let mut n = MIN_FFT_SIZE;
        while n <= MAX_FFT_SIZE {
            let bin = (1000.0 * f64::from(n) / f64::from(FS)).round();
            let freq = bin * f64::from(FS) / f64::from(n);
            let len = 4 * n as usize;
            for (level_dbfs, want) in [(-20.0, -20.0), (0.0, 0.0)] {
                let signal = sine(freq, level_dbfs, len);
                let codes = frame_at(&signal, len as i64 / 2, n, n / 16, false);
                assert_eq!(peak_bin(&codes), bin as usize, "N = {n}");
                let level = f64::from(dequantize(codes[bin as usize]));
                assert!(
                    (level - want).abs() <= 0.35,
                    "N = {n}, {level_dbfs} dBFS → {level}"
                );
            }
            n *= 2;
        }
    }

    /// SPEC-007 AC-2: seeded white noise at −20 dBFS RMS → mean power of the dequantized bins
    /// 100 Hz–20 kHz over interior frames = −45.33 ± 0.5 dB.
    #[test]
    fn ac2_white_noise_per_bin_convention() {
        let noise = vox_testkit::signal::white_noise(7, -20.0, 10.0, FS).unwrap();
        let (lo, hi) = (
            (100.0 * 2048.0 / f64::from(FS)).ceil() as usize,
            (20_000.0 * 2048.0 / f64::from(FS)).floor() as usize,
        );
        let mut sum = 0.0f64;
        let mut count = 0usize;
        let mut center = 2048i64;
        while center + 2048 < noise.len() as i64 {
            let codes = frame_at(&noise, center, 2048, 2048, false);
            for &c in &codes[lo..=hi] {
                sum += 10f64.powf(f64::from(dequantize(c)) / 10.0);
                count += 1;
            }
            center += 2048;
        }
        let mean_db = 10.0 * (sum / count as f64).log10();
        let want = -20.0 + 10.0 * (6.0f64 / 2048.0).log10();
        assert!((mean_db - want).abs() <= 0.5, "{mean_db} vs {want}");
    }

    /// SPEC-007 AC-3: silence → code 0 everywhere; −120 dBFS → −120 ± 1; +3 dBFS → +3 ± 0.35;
    /// +10 dBFS → 255; non-finite samples are analysed as zeros.
    #[test]
    fn ac3_dynamic_range_extremes_and_non_finite_input() {
        let silence = vec![0.0f32; 8192];
        assert!(
            frame_at(&silence, 4096, 2048, 128, false)
                .iter()
                .all(|&c| c == 0)
        );

        let freq = 43.0 * f64::from(FS) / 2048.0;
        for (level_dbfs, tol) in [(-120.0, 1.0), (3.0, 0.35)] {
            let s = sine(freq, level_dbfs, 8192);
            let level = f64::from(dequantize(frame_at(&s, 4096, 2048, 128, false)[43]));
            assert!((level - level_dbfs).abs() <= tol, "{level_dbfs} → {level}");
        }
        let loud = sine(freq, 10.0, 8192);
        assert_eq!(frame_at(&loud, 4096, 2048, 128, false)[43], 255);

        let mut dirty = sine(freq, -20.0, 8192);
        let mut clean = dirty.clone();
        for (i, bad) in [
            (4000, f32::NAN),
            (4100, f32::INFINITY),
            (4200, f32::NEG_INFINITY),
        ] {
            dirty[i] = bad;
            clean[i] = 0.0;
        }
        assert_eq!(
            frame_at(&dirty, 4096, 2048, 128, false),
            frame_at(&clean, 4096, 2048, 128, false)
        );
    }

    /// SPEC-007 AC-5 (first part, frame level): a bin-centred −20 dBFS tone reads −20.0 ± 0.5 dB
    /// in bin 43 at every hop N/16 … 65 536, preview and refined alike.
    #[test]
    fn ac5_stationary_tone_reads_the_same_at_every_hop() {
        let signal = sine(1007.8125, -20.0, 400_000);
        let mut hop = 128u32;
        while hop <= 65_536 {
            for preview in [false, true] {
                let codes = frame_at(&signal, 200_000, 2048, hop, preview);
                let level = dequantize(codes[43]);
                assert!(
                    (level + 20.0).abs() <= 0.5,
                    "hop {hop} preview {preview}: {level}"
                );
            }
            hop *= 2;
        }
    }

    /// SPEC-007 AC-5 (second part): pink noise's mean power over 100 Hz–10 kHz differs by ≤ 0.5 dB
    /// between hop 512 and refined hop 65 536 (mean power keeps stationary levels zoom-invariant).
    #[test]
    fn ac5_pink_noise_mean_power_is_zoom_invariant() {
        let noise = vox_testkit::signal::pink_noise(3, -20.0, 60.0, FS).unwrap();
        let (lo, hi) = (
            (100.0 * 2048.0 / f64::from(FS)).ceil() as usize,
            (10_000.0 * 2048.0 / f64::from(FS)).floor() as usize,
        );
        let mean_db = |hop: u32| {
            let mut sum = 0.0f64;
            let mut count = 0usize;
            let margin = i64::from(hop) + 4096;
            let mut center = margin;
            while center + margin < noise.len() as i64 {
                let codes = frame_at(&noise, center, 2048, hop, false);
                for &c in &codes[lo..=hi] {
                    sum += 10f64.powf(f64::from(dequantize(c)) / 10.0);
                    count += 1;
                }
                center += i64::from(hop.max(2048));
            }
            10.0 * (sum / count as f64).log10()
        };
        let plain = mean_db(512);
        let refined = mean_db(65_536);
        assert!((plain - refined).abs() <= 0.5, "{plain} vs {refined}");
    }

    /// Hann at stride N/2 is COLA: the refined sub-frames of an overview frame cover its input
    /// evenly, so a DC offset's power reads the same as a plain frame's (window/overlap
    /// normalization).
    #[test]
    fn overview_sub_frames_are_normalized_like_plain_frames() {
        let fi = frame_input(2048, 8192, false);
        assert_eq!(fi.sub_frames, 8);
        assert_eq!(fi.len, 8192 + 1024);
        assert_eq!(fi.offset, -(4096 + 512));
        // Sub-frame centres: c − hop/2 + (j + ½)·N/2.
        for j in 0..fi.sub_frames as i64 {
            let first_sample = fi.offset + j * 1024;
            assert_eq!(first_sample + 1024, -4096 + (2 * j + 1) * 512);
        }
        // DC offset A reads 20·log10(2A) in bin 0 at every hop, plain or refined.
        let dc = vec![0.25f32; 100_000];
        for hop in [128u32, 2048, 8192, 32_768] {
            let level = dequantize(frame_at(&dc, 50_000, 2048, hop, false)[0]);
            assert!(
                (level - (20.0 * 0.5f32.log10())).abs() <= 0.35,
                "hop {hop}: {level}"
            );
        }
    }

    /// SPEC-007 AC-6 (hop rule) and AC-14 (Auto table).
    #[test]
    fn hop_rule_and_auto_fft_size() {
        let mut n = MIN_FFT_SIZE;
        while n <= MAX_FFT_SIZE {
            let mut spp = 0.1f64;
            while spp <= 2e5 {
                let hop = hop_for_zoom(spp, n);
                assert!(hop.is_power_of_two() && is_valid_hop(n, hop));
                assert!(hop >= n / 16);
                if spp >= 1.0 && f64::from(hop) > f64::from(n / 16) {
                    // pow2_floor: hop ≤ spp < 2·hop → 1–2 frames per device column.
                    assert!(
                        f64::from(hop) <= spp && spp < 2.0 * f64::from(hop),
                        "{spp} → {hop}"
                    );
                }
                spp *= 1.07;
            }
            n *= 2;
        }
        assert_eq!(hop_for_zoom(1000.0, 2048), 512);
        assert_eq!(hop_for_zoom(0.1, 2048), 128);
        assert_eq!(hop_for_zoom(f64::NAN, 2048), 128);
        assert_eq!(hop_for_zoom(90_000.0, 2048), 65_536);

        assert_eq!(auto_fft_size(44_100), 2048);
        assert_eq!(auto_fft_size(48_000), 2048);
        assert_eq!(auto_fft_size(88_200), 4096);
        assert_eq!(auto_fft_size(96_000), 4096);
        assert_eq!(auto_fft_size(176_400), 8192);
        assert_eq!(auto_fft_size(192_000), 8192);
        assert_eq!(auto_fft_size(22_050), 1024);
        assert_eq!(auto_fft_size(24_000), 1024);
        assert_eq!(auto_fft_size(8_000), 256);
        assert!(is_valid_fft_size(256) && is_valid_fft_size(16_384));
        assert!(!is_valid_fft_size(128) && !is_valid_fft_size(32_768) && !is_valid_fft_size(1000));
    }
}

//! Deterministic TPDF dither/quantizer for integer WAV and FLAC writes (SPEC-005 §2.8, §4.4;
//! ADR-001 §4: TPDF dither lives in `dsp`).
//!
//! `vox-io`'s WAV and FLAC writers call [`quantize_dithered`] (whole buffer at once) or
//! [`StreamingQuantizer`] (fed a block at a time, so a save never needs the whole document in
//! RAM — H-02) for their 16/24-bit output. Both share the same RNG/algorithm, so a save streamed
//! in blocks produces byte-identical output to one that quantizes the whole buffer in one call,
//! given the same total samples and seed.

/// A block of samples that is already exactly representable at the target bit depth (an unedited
/// run from a file already at that depth, or digital silence) is written **without** dither, so
/// open→save round-trips bit-exactly and digital silence stays digital silence (SPEC-005 §2.8).
/// Every other block is dithered in full.
pub const DITHER_BLOCK_SAMPLES: usize = 4096;

/// H-20 (SPEC-005 §2.7/§2.8, `save_dither` parameter): the dither mode Save As offers for 16/24-bit
/// targets, remembered as a preference. `Tpdf` is the SPEC-005 default; `None` rounds half away
/// from zero with no added noise, for every sample (not only off-grid ones) — exactly what
/// `vox_testkit::encode_int_sample` does, so a `None`-dithered save matches testkit's own encoder
/// bit-exactly (AC-4). 32-bit float targets never dither regardless of this mode (never
/// constructed with a [`StreamingQuantizer`] at all).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DitherMode {
    #[default]
    Tpdf,
    None,
}

/// PCG32 XSH-RR (O'Neill 2014), the same construction as `vox_testkit::prng::Pcg32` but
/// reimplemented here: `dsp` must not pull in `testkit` outside `dev-dependencies` (ADR-001 §3,
/// §6). Fixed seed and stream, reset at the start of every save, so saving the same document
/// twice is bit-identical (SPEC-005 §2.8 "Save is a pure function of snapshot + format").
struct DitherRng {
    state: u64,
    inc: u64,
}

const PCG_MULTIPLIER: u64 = 6_364_136_223_846_793_005;
/// Fixed seed/stream constants for the save dither generator (SPEC-005 §2.8). Arbitrary but fixed:
/// any two odd/any values would do, chosen here to be recognizable in a hex dump.
const DITHER_SEED: u64 = 0x504f_5765_5254_5044;
const DITHER_STREAM: u64 = 0x5644_4954_4845_5230;

impl DitherRng {
    fn new() -> Self {
        let mut rng = DitherRng {
            state: 0,
            inc: (DITHER_STREAM << 1) | 1,
        };
        let _ = rng.step();
        rng.state = rng.state.wrapping_add(DITHER_SEED);
        let _ = rng.step();
        rng
    }

    fn step(&mut self) -> u64 {
        let old_state = self.state;
        self.state = old_state
            .wrapping_mul(PCG_MULTIPLIER)
            .wrapping_add(self.inc);
        old_state
    }

    fn next_u32(&mut self) -> u32 {
        let old_state = self.step();
        let xorshifted = (((old_state >> 18) ^ old_state) >> 27) as u32;
        let rot = (old_state >> 59) as u32;
        xorshifted.rotate_right(rot)
    }

    /// Uniform in `[0, 1)` LSB.
    fn next_unit(&mut self) -> f64 {
        f64::from(self.next_u32()) / (f64::from(u32::MAX) + 1.0)
    }
}

/// `true` if `x` scaled by `scale` (a power of two, so the multiply is exact) is already an
/// integer: `x` is exactly representable at this bit depth (SPEC-005 §2.8 "grid-exact").
#[allow(clippy::float_cmp)] // deliberate exact-value check: multiplying by a power of two is exact
fn is_grid_exact(x: f32, scale: f32) -> bool {
    let scaled = x * scale;
    scaled.is_finite() && scaled == scaled.round()
}

/// Quantizes `samples` to signed `bits`-bit integers (16 or 24), dithering every
/// [`DITHER_BLOCK_SAMPLES`]-sample block that isn't already grid-exact at `bits` with TPDF noise
/// (`d = u1 - u2`, independent uniforms in `[0, 1)` LSB — SPEC-005 §2.8), then rounding half away
/// from zero and clamping to the integer range. Returns the quantized values and the number of
/// input samples whose magnitude exceeded 1.0 (clipped).
///
/// Equivalent to feeding the whole buffer to a single-call [`StreamingQuantizer`]; this is the
/// convenient whole-buffer form callers that already hold everything in memory (FLAC export) use.
pub fn quantize_dithered(samples: &[f32], bits: u32, mode: DitherMode) -> (Vec<i32>, usize) {
    let mut q = StreamingQuantizer::new(bits, mode);
    let mut out = Vec::with_capacity(samples.len());
    let clipped = q.push(samples, &mut out);
    (out, clipped)
}

/// Streaming counterpart of [`quantize_dithered`] (H-02): the RNG persists across [`Self::push`]
/// calls, so a caller can feed a long document a block at a time — never holding the whole thing
/// in memory — and still get the exact same output as quantizing it all in one call.
///
/// **Contract:** dither blocks are [`DITHER_BLOCK_SAMPLES`]-sample windows of the overall
/// stream, not of a single `push` call. Every call except optionally the last (the final,
/// possibly-short tail of the document) must therefore push a whole multiple of
/// `DITHER_BLOCK_SAMPLES` samples, so block boundaries land the same way they would over one
/// contiguous buffer. A fixed block size that is itself a multiple of `DITHER_BLOCK_SAMPLES`
/// (e.g. `vox_project::CHUNK_SAMPLES` = 65 536 = 16 × 4096) satisfies this by construction.
/// Debug builds assert the entry alignment.
pub struct StreamingQuantizer {
    rng: DitherRng,
    bits: u32,
    mode: DitherMode,
    /// Total samples pushed so far, tracked only for the debug-build alignment assertion.
    pos: u64,
}

impl StreamingQuantizer {
    /// A fresh quantizer for `bits`-bit output (16 or 24) in `mode`, RNG reset to the fixed seed
    /// (SPEC-005 §2.8: reset at the start of every save) — the RNG is always constructed, even in
    /// [`DitherMode::None`] (where it is simply never drawn from), so switching modes never
    /// changes the PRNG's state machine.
    pub fn new(bits: u32, mode: DitherMode) -> Self {
        debug_assert!(bits == 16 || bits == 24);
        StreamingQuantizer {
            rng: DitherRng::new(),
            bits,
            mode,
            pos: 0,
        }
    }

    /// Quantizes `samples`, appending their quantized values to `out` (`out` is not cleared
    /// first, so a caller can reuse one scratch `Vec` across calls after draining it). Returns
    /// the number of inputs in this call whose magnitude exceeded 1.0 (clipped).
    pub fn push(&mut self, samples: &[f32], out: &mut Vec<i32>) -> usize {
        debug_assert_eq!(
            self.pos % DITHER_BLOCK_SAMPLES as u64,
            0,
            "StreamingQuantizer::push: every call but the last must align to a \
             DITHER_BLOCK_SAMPLES boundary"
        );
        let scale = (1i64 << (self.bits - 1)) as f32;
        let max = (1i64 << (self.bits - 1)) - 1;
        let min = -(1i64 << (self.bits - 1));
        let mut clipped = 0usize;
        out.reserve(samples.len());
        for block in samples.chunks(DITHER_BLOCK_SAMPLES) {
            // `DitherMode::None` (SPEC-005 §2.8 "Dither None rounds half away from zero") applies
            // plain rounding to *every* sample uniformly — grid-exactness is irrelevant (an exact
            // value rounds to itself either way) and the RNG is never drawn from, so a `None` save
            // draws zero PRNG samples regardless of length.
            let grid_exact =
                self.mode == DitherMode::None || block.iter().all(|&x| is_grid_exact(x, scale));
            for &x in block {
                if x.abs() > 1.0 {
                    clipped += 1;
                }
                let q = if grid_exact {
                    (f64::from(x) * f64::from(scale)).round() as i64
                } else {
                    let d = self.rng.next_unit() - self.rng.next_unit();
                    (f64::from(x) * f64::from(scale) + d).round() as i64
                };
                out.push(q.clamp(min, max) as i32);
            }
        }
        self.pos += samples.len() as u64;
        clipped
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_exact_16_bit_values_round_trip_without_dither() {
        // Every value already an exact 16-bit grid point (i / 32768 for i in range).
        let samples: Vec<f32> = (-100..100).map(|i| i as f32 / 32_768.0).collect();
        let (q, clipped) = quantize_dithered(&samples, 16, DitherMode::Tpdf);
        assert_eq!(clipped, 0);
        let back: Vec<i32> = (-100..100).collect();
        assert_eq!(q, back, "grid-exact block must not be dithered");
    }

    #[test]
    fn silence_stays_silence() {
        let samples = vec![0.0f32; DITHER_BLOCK_SAMPLES * 3];
        let (q, clipped) = quantize_dithered(&samples, 16, DitherMode::Tpdf);
        assert_eq!(clipped, 0);
        assert!(q.iter().all(|&v| v == 0));
    }

    #[test]
    fn deterministic_across_calls() {
        let samples: Vec<f32> = (0..10_000)
            .map(|i| (i as f32 * 0.0137).sin() * 0.6)
            .collect();
        let (a, _) = quantize_dithered(&samples, 16, DitherMode::Tpdf);
        let (b, _) = quantize_dithered(&samples, 16, DitherMode::Tpdf);
        assert_eq!(
            a, b,
            "the RNG resets every call, so output is deterministic"
        );
    }

    #[test]
    fn out_of_range_samples_are_counted_and_clamped() {
        let samples = [1.5f32, -2.0, 0.0];
        let (q, clipped) = quantize_dithered(&samples, 16, DitherMode::Tpdf);
        assert_eq!(clipped, 2);
        assert_eq!(q[0], ((1i64 << 15) - 1) as i32);
        assert_eq!(q[1], -(1i64 << 15) as i32);
    }

    #[test]
    fn dither_stays_within_plus_minus_one_lsb_of_rounding() {
        // Off-grid values (24-bit grid, quantized at 16-bit): dithered output must still land
        // within 1 LSB of plain rounding (TPDF noise is bounded to +/-1 LSB).
        let samples: Vec<f32> = (0..DITHER_BLOCK_SAMPLES)
            .map(|i| ((i as f32) * 0.3).sin() * 0.7 + 1.0 / 16_777_216.0)
            .collect();
        let (q, _) = quantize_dithered(&samples, 16, DitherMode::Tpdf);
        for (i, (&x, &v)) in samples.iter().zip(q.iter()).enumerate() {
            let plain = (f64::from(x) * 32_768.0).round() as i64;
            assert!(
                (i64::from(v) - plain).abs() <= 1,
                "sample {i}: dithered {v} vs plain-rounded {plain}"
            );
        }
    }

    /// H-02: a document fed to `StreamingQuantizer` in several calls (each a multiple of
    /// `DITHER_BLOCK_SAMPLES`, like `save_snapshot_wav`'s `CHUNK_SAMPLES` blocks) must produce
    /// exactly the same output as `quantize_dithered` over the whole buffer at once.
    #[test]
    fn streaming_in_blocks_matches_one_shot_quantization() {
        let n = DITHER_BLOCK_SAMPLES * 20 + 777; // several full blocks plus a short tail
        let samples: Vec<f32> = (0..n)
            .map(|i| match i % 7 {
                0 => 0.0,                                                           // silence
                1 => ((i * 37) % 65536) as f32 / 32_768.0 - 1.0, // exact 16-bit grid point
                _ => ((i as f32 * 12.9898).sin() * 43_758.547).fract() * 1.6 - 0.8, // off-grid
            })
            .collect();

        let (one_shot, one_shot_clipped) = quantize_dithered(&samples, 16, DitherMode::Tpdf);

        // Feed it through in CHUNK_SAMPLES-sized blocks (65 536 = 16 * DITHER_BLOCK_SAMPLES), the
        // same block size `save_snapshot_wav` uses, so the last block is short and unaligned.
        const CHUNK_SAMPLES: usize = 65_536;
        let mut streaming = StreamingQuantizer::new(16, DitherMode::Tpdf);
        let mut out = Vec::new();
        let mut streaming_clipped = 0usize;
        for block in samples.chunks(CHUNK_SAMPLES) {
            streaming_clipped += streaming.push(block, &mut out);
        }

        assert_eq!(streaming_clipped, one_shot_clipped);
        assert_eq!(out, one_shot);
    }

    // --- H-20: DitherMode::None -------------------------------------------------------------

    /// SPEC-005 §2.8: "Dither None rounds half away from zero" — for *every* sample uniformly,
    /// not only the grid-exact ones (unlike TPDF, which only skips dither when the whole block is
    /// grid-exact). Off-grid values here would be dithered under TPDF; under `None` they land
    /// exactly on plain rounding instead — this is also testkit's `encode_int_sample` rule (AC-4).
    #[test]
    fn dither_none_rounds_plainly_with_no_added_noise_even_off_grid() {
        let samples: Vec<f32> = (0..DITHER_BLOCK_SAMPLES)
            .map(|i| ((i as f32) * 0.3).sin() * 0.7 + 1.0 / 16_777_216.0)
            .collect();
        let (q, _) = quantize_dithered(&samples, 16, DitherMode::None);
        for (i, (&x, &v)) in samples.iter().zip(q.iter()).enumerate() {
            let plain = (f64::from(x) * 32_768.0).round() as i64;
            assert_eq!(
                i64::from(v),
                plain,
                "sample {i}: None-dithered {v} != plain-rounded {plain}"
            );
        }
    }

    /// Half-LSB boundaries round half away from zero (not to even), at 16-bit scale.
    #[test]
    fn dither_none_rounds_half_lsb_boundaries_away_from_zero() {
        let half_lsb = 0.5 / 32_768.0;
        let samples = [half_lsb, -half_lsb, 3.0 * half_lsb, -3.0 * half_lsb];
        let (q, clipped) = quantize_dithered(&samples, 16, DitherMode::None);
        assert_eq!(clipped, 0);
        assert_eq!(q, vec![1, -1, 2, -2]);
    }

    /// `DitherMode::None` never draws from the PRNG, so it must add no noise where TPDF would —
    /// the two modes diverge on off-grid content...
    #[test]
    fn dither_none_differs_from_tpdf_on_off_grid_content() {
        let samples: Vec<f32> = (0..DITHER_BLOCK_SAMPLES)
            .map(|i| ((i as f32) * 0.7).sin() * 0.6 + 1.0 / 16_777_216.0)
            .collect();
        let (none, _) = quantize_dithered(&samples, 16, DitherMode::None);
        let (tpdf, _) = quantize_dithered(&samples, 16, DitherMode::Tpdf);
        assert_ne!(
            none, tpdf,
            "TPDF must add dither noise that plain rounding doesn't"
        );
    }

    /// ...and agree on grid-exact content, where TPDF already skips dithering (SPEC-005 §2.8's
    /// grid-exact passthrough applies regardless of the chosen dither mode).
    #[test]
    fn dither_none_matches_tpdf_on_grid_exact_content() {
        let samples: Vec<f32> = (-100..100).map(|i| i as f32 / 32_768.0).collect();
        let (none, _) = quantize_dithered(&samples, 16, DitherMode::None);
        let (tpdf, _) = quantize_dithered(&samples, 16, DitherMode::Tpdf);
        assert_eq!(none, tpdf);
    }

    /// H-02's streaming contract holds for `None` too: feeding the document through
    /// `StreamingQuantizer` in `CHUNK_SAMPLES`-sized blocks matches one whole-buffer call.
    #[test]
    fn streaming_none_mode_matches_one_shot_quantization() {
        let n = DITHER_BLOCK_SAMPLES * 5 + 321; // several full blocks plus a short tail
        let samples: Vec<f32> = (0..n)
            .map(|i| ((i as f32 * 12.9898).sin() * 43_758.547).fract() * 1.6 - 0.8)
            .collect();
        let (one_shot, one_shot_clipped) = quantize_dithered(&samples, 24, DitherMode::None);

        const CHUNK_SAMPLES: usize = 65_536;
        let mut streaming = StreamingQuantizer::new(24, DitherMode::None);
        let mut out = Vec::new();
        let mut streaming_clipped = 0usize;
        for block in samples.chunks(CHUNK_SAMPLES) {
            streaming_clipped += streaming.push(block, &mut out);
        }

        assert_eq!(streaming_clipped, one_shot_clipped);
        assert_eq!(out, one_shot);
    }

    // --- H-60 (SPEC-005 AC-4): dither removes harmonic distortion, TPDF whitens it ------------

    /// Magnitude spectrum in dB (unnormalized: only relative levels within one call are
    /// meaningful) of `samples` via a single real FFT of `samples.len()` points.
    fn magnitude_spectrum_db(samples: &[i32], scale: f64) -> Vec<f64> {
        let mut planner = realfft::RealFftPlanner::<f64>::new();
        let fft = planner.plan_fft_forward(samples.len());
        let mut input: Vec<f64> = samples.iter().map(|&s| f64::from(s) / scale).collect();
        let mut spectrum = fft.make_output_vec();
        fft.process(&mut input, &mut spectrum).unwrap();
        spectrum
            .iter()
            .map(|c| 20.0 * (c.norm() + 1e-300).log10())
            .collect()
    }

    /// The median dB level of the bins in `[center - window, center + window]`, excluding
    /// `[center - exclude, center + exclude]` (the harmonic bin itself and its immediate
    /// skirt) — the AC-4 "median of the neighbouring bins" reference level.
    fn neighbor_median_db(
        spectrum_db: &[f64],
        center: usize,
        exclude: usize,
        window: usize,
    ) -> f64 {
        let lo = center.saturating_sub(window);
        let hi = (center + window).min(spectrum_db.len() - 1);
        let mut levels: Vec<f64> = (lo..=hi)
            .filter(|&k| k.abs_diff(center) > exclude)
            .map(|k| spectrum_db[k])
            .collect();
        levels.sort_by(|a, b| a.partial_cmp(b).unwrap());
        levels[levels.len() / 2]
    }

    /// SPEC-005 AC-4: a low-level sine, saved at 16-bit with **Dither None**, shows a clearly
    /// isolated harmonic (undithered rounding is a nonlinearity: it distorts, it doesn't add
    /// noise) — but saved with **TPDF**, the same harmonic bins read within a few dB of their
    /// surrounding noise floor, because TPDF replaces that correlated distortion with
    /// signal-independent broadband dither noise. This is the one AC that proves dither does what
    /// it's for, rather than only checking bulk residual statistics (AC-3) or bit-level bounds.
    ///
    /// Deliberately simplified from the spec's literal methodology (8192-point Blackman-Harris
    /// averaged over 10 s of real 1000/2000/3000/5000 Hz tones) to one bin-centred tone and a
    /// single rectangular-window 8192-point FFT frame — bin-centred so there is no spectral
    /// leakage to account for without a taper window (`docs/references.md`'s "ACs use bin-centred
    /// tones" convention), and a single frame because the effect this test checks for (a
    /// stationary nonlinearity's harmonics vs. dither's white floor) doesn't need averaging to
    /// show up cleanly at 8192 points. The full spec-literal AC-4 harness (real tones, BH4,
    /// 10 s average) is a candidate for a future `testkit` spectral-analysis helper (none exists
    /// yet in this workspace — every crate that needs one currently hand-rolls it, see
    /// `crates/io/tests/codec_decode.rs::dominant_frequency_hz`); proposed as a follow-up rather
    /// than grown here, since a shared helper touches every consumer's test conventions.
    #[test]
    fn tpdf_masks_harmonic_distortion_that_dither_none_leaves_isolated() {
        const N: usize = 8192;
        const RATE_HZ: f64 = 48_000.0;
        // Bin-centred fundamental near 1000 Hz (bin 171 of 8192 @ 48 kHz = 1001.953125 Hz), so its
        // harmonics (2x/3x/5x) land on exact bins too, with zero leakage under a rectangular
        // window.
        const K0: usize = 171;
        let f0_hz = K0 as f64 * RATE_HZ / N as f64;
        // -80 dBFS peak, "≈3.3 LSB at 16-bit" per SPEC-005 AC-4.
        let amp = 10f64.powf(-80.0 / 20.0);
        let samples: Vec<f32> = (0..N)
            .map(|i| (amp * (2.0 * std::f64::consts::PI * f0_hz * i as f64 / RATE_HZ).sin()) as f32)
            .collect();

        let (none, _) = quantize_dithered(&samples, 16, DitherMode::None);
        let (tpdf, _) = quantize_dithered(&samples, 16, DitherMode::Tpdf);
        let scale = f64::from(1u32 << 15);
        let none_db = magnitude_spectrum_db(&none, scale);
        let tpdf_db = magnitude_spectrum_db(&tpdf, scale);

        // AC-4's own worked example: with Dither None, the 3 kHz component (3rd harmonic, bin
        // 3*K0) is >= 20 dB above the median of its neighbouring band.
        let third = 3 * K0;
        let none_excess = none_db[third] - neighbor_median_db(&none_db, third, 3, 40);
        assert!(
            none_excess >= 20.0,
            "Dither None: 3rd-harmonic bin is only {none_excess:.1} dB above its neighbours \
             (want >= 20 dB, i.e. a clearly isolated distortion line)"
        );

        // With TPDF, the 2nd/3rd/5th harmonics are each within a few dB of their local noise
        // floor -- no longer an isolated spike. (The spec's own "+3 dB" bound is for a full 10 s
        // BH4 average; a single 8192-point frame's per-bin noise estimate is noisier, so this
        // uses a slightly wider tolerance appropriate to one frame.)
        for &k in &[2 * K0, 3 * K0, 5 * K0] {
            let excess = tpdf_db[k] - neighbor_median_db(&tpdf_db, k, 3, 40);
            assert!(
                excess <= 6.0,
                "TPDF: harmonic bin {k} is {excess:.1} dB above its neighbours (want <= 6 dB, \
                 i.e. masked into the dither floor, not left as a distortion line)"
            );
        }

        // And the isolated-harmonic gap must actually close under TPDF vs. None at the same bin.
        let tpdf_third_excess = tpdf_db[third] - neighbor_median_db(&tpdf_db, third, 3, 40);
        assert!(
            tpdf_third_excess < none_excess - 10.0,
            "TPDF should suppress the 3rd harmonic's isolation by at least 10 dB vs. Dither \
             None (None {none_excess:.1} dB, TPDF {tpdf_third_excess:.1} dB)"
        );
    }
}

//! Deterministic TPDF dither for integer WAV writes (SPEC-005 §2.8).
//!
//! ADR-001 §4 lists TPDF dither as `dsp`'s concern; this ticket's scope (S1-02) is `vox-io` and
//! `vox-project` only, and `dsp` is still an empty skeleton, so the quantizer lives here for now.
//! Moving it into `vox_dsp::dither` (with `io` calling through it) is a small follow-up once `dsp`
//! has its first real content — noted in the ticket report.

/// A block of samples that is already exactly representable at the target bit depth (an unedited
/// run from a file already at that depth, or digital silence) is written **without** dither, so
/// open→save round-trips bit-exactly and digital silence stays digital silence (SPEC-005 §2.8).
/// Every other block is dithered in full.
pub(crate) const DITHER_BLOCK_SAMPLES: usize = 4096;

/// PCG32 XSH-RR (O'Neill 2014), the same construction as `vox_testkit::prng::Pcg32` but
/// reimplemented here: `io` must not pull in `testkit` outside `dev-dependencies` (ADR-001 §3, §6).
/// Fixed seed and stream, reset at the start of every write, so saving the same document twice is
/// bit-identical (SPEC-005 §2.8 "Save is a pure function of snapshot + format").
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
pub(crate) fn quantize_dithered(samples: &[f32], bits: u32) -> (Vec<i32>, usize) {
    debug_assert!(bits == 16 || bits == 24);
    let scale = (1i64 << (bits - 1)) as f32;
    let max = (1i64 << (bits - 1)) - 1;
    let min = -(1i64 << (bits - 1));
    let mut rng = DitherRng::new();
    let mut out = Vec::with_capacity(samples.len());
    let mut clipped = 0usize;
    for block in samples.chunks(DITHER_BLOCK_SAMPLES) {
        let grid_exact = block.iter().all(|&x| is_grid_exact(x, scale));
        for &x in block {
            if x.abs() > 1.0 {
                clipped += 1;
            }
            let q = if grid_exact {
                (f64::from(x) * f64::from(scale)).round() as i64
            } else {
                let d = rng.next_unit() - rng.next_unit();
                (f64::from(x) * f64::from(scale) + d).round() as i64
            };
            out.push(q.clamp(min, max) as i32);
        }
    }
    (out, clipped)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_exact_16_bit_values_round_trip_without_dither() {
        // Every value already an exact 16-bit grid point (i / 32768 for i in range).
        let samples: Vec<f32> = (-100..100).map(|i| i as f32 / 32_768.0).collect();
        let (q, clipped) = quantize_dithered(&samples, 16);
        assert_eq!(clipped, 0);
        let back: Vec<i32> = (-100..100).collect();
        assert_eq!(q, back, "grid-exact block must not be dithered");
    }

    #[test]
    fn silence_stays_silence() {
        let samples = vec![0.0f32; DITHER_BLOCK_SAMPLES * 3];
        let (q, clipped) = quantize_dithered(&samples, 16);
        assert_eq!(clipped, 0);
        assert!(q.iter().all(|&v| v == 0));
    }

    #[test]
    fn deterministic_across_calls() {
        let samples: Vec<f32> = (0..10_000)
            .map(|i| (i as f32 * 0.0137).sin() * 0.6)
            .collect();
        let (a, _) = quantize_dithered(&samples, 16);
        let (b, _) = quantize_dithered(&samples, 16);
        assert_eq!(
            a, b,
            "the RNG resets every call, so output is deterministic"
        );
    }

    #[test]
    fn out_of_range_samples_are_counted_and_clamped() {
        let samples = [1.5f32, -2.0, 0.0];
        let (q, clipped) = quantize_dithered(&samples, 16);
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
        let (q, _) = quantize_dithered(&samples, 16);
        for (i, (&x, &v)) in samples.iter().zip(q.iter()).enumerate() {
            let plain = (f64::from(x) * 32_768.0).round() as i64;
            assert!(
                (i64::from(v) - plain).abs() <= 1,
                "sample {i}: dithered {v} vs plain-rounded {plain}"
            );
        }
    }
}

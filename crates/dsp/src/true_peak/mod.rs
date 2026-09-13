//! True-peak limiter DSP (SPEC-017 §4.1–§4.3, Amendment 2). f64 inside; the module converts at
//! the edges.
//!
//! ```text
//! u ─┬──────────────────── delay L + D + 1 ─────────────────────► × G ─► y
//!    └► 4× TP detector (D) ─► r ─► ρ = min(r, r₋₁) ─► min-hold (L+1) ─► release ─► 2× avg ─► G
//! ```
//!
//! - [`TruePeakDetector`]: 4× polyphase interpolator + parabolic refinement, D = 16 samples.
//! - Per-sample requirement ρ (H-03): input sample n bounds two intervals, (n − 1, n) and
//!   (n, n + 1); ρ[n] is the lower of their requirements, known one sample after the second
//!   interval's detector reading. So every output sample's gain honours both intervals it
//!   bounds **exactly** (by construction, on the detector's estimate), at the cost of one more
//!   sample of latency.
//! - [`LookaheadGain`]: hold, linear-in-dB release, two moving averages (exact unity at rest).
//! - [`TruePeakLimiterCore`]: all of it, plus the audio delay line and the per-sample ceiling
//!   that travels with the audio (so a sample is judged against the ceiling in force when it is
//!   output).

mod detector;
mod gain;

pub use detector::{DETECTOR_DELAY, KAISER_BETA, OVERSAMPLING, TAPS_PER_PHASE, TruePeakDetector};
pub use gain::LookaheadGain;

/// Gain recovered per release time, in dB (Audition's "time to rebound 12 dB").
pub const RELEASE_DB_PER_TIME: f64 = 12.0;

/// Delay from an input sample to its per-sample requirement ρ: the detector's D plus one sample,
/// so that both intervals the sample bounds are known (SPEC-017 Amendment 2).
pub const REQUIREMENT_DELAY: usize = DETECTOR_DELAY + 1;

/// Look-ahead L in samples: `round(ms · fs / 1000)`, rounded up to even.
pub fn lookahead_samples(lookahead_ms: f64, sample_rate: f64) -> usize {
    let l = (lookahead_ms * sample_rate / 1000.0).round().max(0.0) as usize;
    l + (l & 1)
}

/// Latency of a limiter with look-ahead `lookahead` samples: L + D + 1
/// ([`REQUIREMENT_DELAY`]); 257 at 48 kHz / 5 ms.
pub fn latency_samples(lookahead: usize) -> usize {
    lookahead + REQUIREMENT_DELAY
}

/// Per-sample release factor `k_r = 10^(12 / (20 · release_s · fs))`.
pub fn release_coeff(release_ms: f64, sample_rate: f64) -> f64 {
    10f64.powf(RELEASE_DB_PER_TIME / (20.0 * release_ms / 1000.0 * sample_rate))
}

/// Ring of per-sample ceilings; a power of two > D + 1.
const CEILING_RING: usize = 32;
const CEILING_MASK: usize = CEILING_RING - 1;

/// The complete limiter for one channel: `u` and its ceiling in, `y = G · u[n − L − D − 1]` out.
#[derive(Clone, Debug)]
pub struct TruePeakLimiterCore {
    detector: TruePeakDetector,
    gain: LookaheadGain,
    delay: Box<[f64]>,
    delay_pos: usize,
    ceiling: [f64; CEILING_RING],
    /// Requirement of the previous interval (the one ending one sample earlier).
    prev_r: f64,
    t: u64,
}

impl TruePeakLimiterCore {
    /// A limiter with look-ahead `lookahead` samples (see [`lookahead_samples`]), release factor
    /// `release_coeff` (see [`release_coeff`]) and initial linear ceiling. Allocates.
    pub fn new(lookahead: usize, release_coeff: f64, ceiling_lin: f64) -> Self {
        Self {
            detector: TruePeakDetector::new(),
            gain: LookaheadGain::new(lookahead, release_coeff),
            delay: vec![0.0; latency_samples(lookahead)].into_boxed_slice(),
            delay_pos: 0,
            ceiling: [ceiling_lin; CEILING_RING],
            prev_r: 1.0,
            t: 0,
        }
    }

    /// Look-ahead L in samples.
    pub fn lookahead(&self) -> usize {
        self.gain.lookahead()
    }

    /// Latency L + D + 1 in samples (see [`latency_samples`]).
    pub fn latency(&self) -> usize {
        self.delay.len()
    }

    /// Sets the release factor for the gain computed from the next sample on. RT-safe.
    pub fn set_release_coeff(&mut self, k: f64) {
        self.gain.set_release_coeff(k);
    }

    /// Clears the delay line, detector, hold, release and averages (G = 1); every stored ceiling
    /// becomes `ceiling_lin`. RT-safe.
    pub fn reset(&mut self, ceiling_lin: f64) {
        self.detector.reset();
        self.gain.reset();
        self.delay.fill(0.0);
        self.delay_pos = 0;
        self.ceiling = [ceiling_lin; CEILING_RING];
        self.prev_r = 1.0;
    }

    /// Processes one sample: `u` (after input gain) with the linear ceiling attached to it.
    /// Returns `(y, G)`: the output sample (input from L + D + 1 samples ago times G) and the
    /// gain applied to it. RT-safe.
    #[inline]
    pub fn process(&mut self, u: f64, ceiling_lin: f64) -> (f64, f64) {
        let t = self.t as usize;
        self.t += 1;
        self.ceiling[t & CEILING_MASK] = ceiling_lin;
        // Interval (t − 17, t − 16), judged against the lower of its endpoints' ceilings.
        let q = self.detector.push(u);
        let c = self.ceiling[t.wrapping_sub(DETECTOR_DELAY) & CEILING_MASK]
            .min(self.ceiling[t.wrapping_sub(DETECTOR_DELAY + 1) & CEILING_MASK]);
        let r = if q > c { c / q } else { 1.0 };
        // Sample t − 17 bounds (t − 18, t − 17) and (t − 17, t − 16): its gain honours both.
        let rho = r.min(self.prev_r);
        self.prev_r = r;
        let g = self.gain.push(rho);
        let delayed = std::mem::replace(&mut self.delay[self.delay_pos], u);
        self.delay_pos += 1;
        if self.delay_pos == self.delay.len() {
            self.delay_pos = 0;
        }
        (g * delayed, g)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::float_cmp)] // bit-exact transparency is the property under test
    use super::*;

    #[test]
    fn lookahead_table() {
        // (rate, ms) → L + D + 1, SPEC-017 AC-2 (Amendment 2: exact endpoint coverage).
        let cases = [
            (44_100.0, 1.0, 61),
            (44_100.0, 5.0, 239),
            (44_100.0, 10.0, 459),
            (48_000.0, 1.0, 65),
            (48_000.0, 5.0, 257),
            (48_000.0, 10.0, 497),
            (96_000.0, 1.0, 113),
            (96_000.0, 5.0, 497),
            (96_000.0, 10.0, 977),
        ];
        for (rate, ms, latency) in cases {
            let l = lookahead_samples(ms, rate);
            assert_eq!(l % 2, 0);
            assert_eq!(latency_samples(l), latency, "{rate} Hz, {ms} ms");
            let core = TruePeakLimiterCore::new(l, release_coeff(100.0, rate), 0.5);
            assert_eq!(core.latency(), latency);
        }
    }

    #[test]
    fn below_ceiling_is_bit_exact_after_latency() {
        let mut core = TruePeakLimiterCore::new(240, release_coeff(100.0, 48_000.0), 0.89);
        let x: Vec<f64> = (0..5_000)
            .map(|n| f64::from((0.8 * (f64::from(n) * 0.05).sin()) as f32))
            .collect();
        let y: Vec<(f64, f64)> = x.iter().map(|&u| core.process(u, 0.89)).collect();
        for n in 257..x.len() {
            assert_eq!(y[n].0, x[n - 257]);
            assert_eq!(y[n].1, 1.0);
        }
    }

    /// H-03: every output sample's gain honours the requirement of **both** intervals it bounds,
    /// (s − 1, s) and (s, s + 1), exactly — not only the one ending at it.
    #[test]
    fn gain_covers_both_adjacent_intervals_exactly() {
        let c = 10f64.powf(-1.0 / 20.0);
        let mut s = 0x5EED_u64;
        let mut next = move || {
            s = s
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (s >> 11) as f64 / (1u64 << 53) as f64 - 0.5
        };
        // Sparse sharp clicks over quiet noise: requirements change from one interval to the next.
        let u: Vec<f64> = (0..40_000)
            .map(|n| {
                let click = if n % 997 < 3 { 3.0 * next() } else { 0.0 };
                0.2 * next() + click
            })
            .collect();
        for lookahead in [48, 240] {
            let mut core = TruePeakLimiterCore::new(lookahead, release_coeff(10.0, 48_000.0), c);
            let latency = core.latency();
            assert_eq!(latency, lookahead + REQUIREMENT_DELAY);
            let g: Vec<f64> = u.iter().map(|&x| core.process(x, c).1).collect();
            // Requirement of the interval ending at sample j (the detector's own reading).
            let mut det = TruePeakDetector::new();
            let q: Vec<f64> = u.iter().map(|&x| det.push(x)).collect();
            let r_end = |j: usize| {
                let q = q[j + DETECTOR_DELAY];
                if q > c { c / q } else { 1.0 }
            };
            for sample in 1..u.len() - latency - 1 {
                let need = r_end(sample).min(r_end(sample + 1));
                let applied = g[sample + latency];
                assert!(
                    applied <= need * (1.0 + 1e-12),
                    "L {lookahead}, sample {sample}: G {applied} > {need}"
                );
            }
        }
    }

    #[test]
    fn sample_peaks_never_exceed_the_ceiling() {
        let c = 10f64.powf(-1.0 / 20.0);
        let mut core = TruePeakLimiterCore::new(48, release_coeff(10.0, 48_000.0), c);
        let mut s = 1u64;
        for n in 0..50_000usize {
            s = s
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            let noise = ((s >> 11) as f64 / (1u64 << 53) as f64 - 0.5) * 16.0;
            let u = if (n / 3_000).is_multiple_of(2) {
                noise
            } else {
                0.01 * noise
            };
            let (y, g) = core.process(u, c);
            assert!(y.abs() <= c * (1.0 + 1e-12), "n {n}: {y}");
            assert!(g > 0.0 && g <= 1.0);
        }
    }
}

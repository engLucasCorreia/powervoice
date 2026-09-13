//! True-peak limiter DSP (SPEC-017 §4.1–§4.3). f64 inside; the module converts at the edges.
//!
//! ```text
//! u ─┬──────────────── delay L + D ─────────────────► × G ─► y
//!    └► 4× TP detector (D) ─► r ─► min-hold (L+1) ─► release ─► 2× moving avg ─► G
//! ```
//!
//! - [`TruePeakDetector`]: 4× polyphase interpolator + parabolic refinement, D = 16 samples.
//! - [`LookaheadGain`]: hold, linear-in-dB release, two moving averages (exact unity at rest).
//! - [`TruePeakLimiterCore`]: both, plus the audio delay line and the per-sample ceiling that
//!   travels with the audio (so a sample is judged against the ceiling in force when it is
//!   output).

mod detector;
mod gain;

pub use detector::{DETECTOR_DELAY, KAISER_BETA, OVERSAMPLING, TAPS_PER_PHASE, TruePeakDetector};
pub use gain::LookaheadGain;

/// Gain recovered per release time, in dB (Audition's "time to rebound 12 dB").
pub const RELEASE_DB_PER_TIME: f64 = 12.0;

/// Look-ahead L in samples: `round(ms · fs / 1000)`, rounded up to even.
pub fn lookahead_samples(lookahead_ms: f64, sample_rate: f64) -> usize {
    let l = (lookahead_ms * sample_rate / 1000.0).round().max(0.0) as usize;
    l + (l & 1)
}

/// Per-sample release factor `k_r = 10^(12 / (20 · release_s · fs))`.
pub fn release_coeff(release_ms: f64, sample_rate: f64) -> f64 {
    10f64.powf(RELEASE_DB_PER_TIME / (20.0 * release_ms / 1000.0 * sample_rate))
}

/// Ring of per-sample ceilings; a power of two > D + 1.
const CEILING_RING: usize = 32;
const CEILING_MASK: usize = CEILING_RING - 1;

/// The complete limiter for one channel: `u` and its ceiling in, `y = G · u[n − L − D]` out.
#[derive(Clone, Debug)]
pub struct TruePeakLimiterCore {
    detector: TruePeakDetector,
    gain: LookaheadGain,
    delay: Box<[f64]>,
    delay_pos: usize,
    ceiling: [f64; CEILING_RING],
    t: u64,
}

impl TruePeakLimiterCore {
    /// A limiter with look-ahead `lookahead` samples (see [`lookahead_samples`]), release factor
    /// `release_coeff` (see [`release_coeff`]) and initial linear ceiling. Allocates.
    pub fn new(lookahead: usize, release_coeff: f64, ceiling_lin: f64) -> Self {
        Self {
            detector: TruePeakDetector::new(),
            gain: LookaheadGain::new(lookahead, release_coeff),
            delay: vec![0.0; lookahead + DETECTOR_DELAY].into_boxed_slice(),
            delay_pos: 0,
            ceiling: [ceiling_lin; CEILING_RING],
            t: 0,
        }
    }

    /// Look-ahead L in samples.
    pub fn lookahead(&self) -> usize {
        self.gain.lookahead()
    }

    /// Latency L + D in samples.
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
    }

    /// Processes one sample: `u` (after input gain) with the linear ceiling attached to it.
    /// Returns `(y, G)`: the output sample (input from L + D samples ago times G) and the gain
    /// applied to it. RT-safe.
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
        let g = self.gain.push(r);
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
        // (rate, ms) → L + D, SPEC-017 AC-2.
        let cases = [
            (44_100.0, 1.0, 60),
            (44_100.0, 5.0, 238),
            (44_100.0, 10.0, 458),
            (48_000.0, 1.0, 64),
            (48_000.0, 5.0, 256),
            (48_000.0, 10.0, 496),
            (96_000.0, 1.0, 112),
            (96_000.0, 5.0, 496),
            (96_000.0, 10.0, 976),
        ];
        for (rate, ms, latency) in cases {
            let l = lookahead_samples(ms, rate);
            assert_eq!(l % 2, 0);
            assert_eq!(l + DETECTOR_DELAY, latency, "{rate} Hz, {ms} ms");
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
        for n in 256..x.len() {
            assert_eq!(y[n].0, x[n - 256]);
            assert_eq!(y[n].1, 1.0);
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

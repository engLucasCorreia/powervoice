//! 4× oversampled true-peak detector with parabolic refinement (SPEC-017 §4.2).
//!
//! - Interpolator: 4× polyphase FIR, [`TAPS_PER_PHASE`] = 32 taps per phase (128 in total),
//!   Kaiser-windowed sinc (cut-off fs/2, window half-width 16 input samples, β =
//!   [`KAISER_BETA`]). Each fractional phase is normalized to unity DC gain. Phase 0 is the input
//!   sample itself, so sample peaks are always seen exactly.
//! - Interval peak: for the interval between input samples j−1 and j, q = max |v| over its five
//!   4× points (both ends included), plus the vertex of the parabola through every local maximum
//!   of |v| among them and its two neighbours.
//! - Delay: the interval ending at input sample j is complete when sample j + [`DETECTOR_DELAY`]
//!   has been pushed.
//!
//! Real-time safe: fixed-size state, no allocation, no branches that depend on the signal
//! besides the refinement.

/// Oversampling factor.
pub const OVERSAMPLING: usize = 4;
/// Taps per polyphase branch.
pub const TAPS_PER_PHASE: usize = 32;
/// Detector delay in input samples (half the window).
pub const DETECTOR_DELAY: usize = TAPS_PER_PHASE / 2;
/// Kaiser window β (≈ 50 dB for the transition 0.4535·fs … 0.5465·fs).
pub const KAISER_BETA: f64 = 4.60;

/// Ring of recent 4× points (|v|); a power of two ≥ 12.
const POINT_RING: usize = 16;
const POINT_MASK: usize = POINT_RING - 1;

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

/// Coefficients of the fractional phases 1/4, 2/4, 3/4. Branch `p` applied to the 32 most recent
/// inputs `w[i] = u[n − 31 + i]` yields the interpolated value at `n − 16 + (p + 1)/4`.
fn phase_coefficients() -> [[f64; TAPS_PER_PHASE]; OVERSAMPLING - 1] {
    let half = DETECTOR_DELAY as f64;
    let mut out = [[0.0; TAPS_PER_PHASE]; OVERSAMPLING - 1];
    for (p, taps) in out.iter_mut().enumerate() {
        let frac = (p + 1) as f64 / OVERSAMPLING as f64;
        for (i, c) in taps.iter_mut().enumerate() {
            let tau = frac + (half - 1.0) - i as f64;
            *c = sinc(tau) * kaiser(tau / half, KAISER_BETA);
        }
        let dc: f64 = taps.iter().sum();
        for c in taps.iter_mut() {
            *c /= dc;
        }
    }
    out
}

/// Streaming 4× true-peak detector: one input sample in, one interval peak out.
#[derive(Clone, Debug)]
pub struct TruePeakDetector {
    coeffs: [[f64; TAPS_PER_PHASE]; OVERSAMPLING - 1],
    /// Input history, written twice so the last 32 samples are always one contiguous slice.
    hist: [f64; 2 * TAPS_PER_PHASE],
    hist_pos: usize,
    /// Recent |v| (4× points), oldest overwritten.
    points: [f64; POINT_RING],
    point_count: usize,
}

impl Default for TruePeakDetector {
    fn default() -> Self {
        Self::new()
    }
}

impl TruePeakDetector {
    /// A detector with cleared history. Does not allocate.
    pub fn new() -> Self {
        Self {
            coeffs: phase_coefficients(),
            hist: [0.0; 2 * TAPS_PER_PHASE],
            hist_pos: 0,
            points: [0.0; POINT_RING],
            point_count: 0,
        }
    }

    /// Clears the history (as if preceded by silence). RT-safe.
    pub fn reset(&mut self) {
        self.hist = [0.0; 2 * TAPS_PER_PHASE];
        self.hist_pos = 0;
        self.points = [0.0; POINT_RING];
        self.point_count = 0;
    }

    fn push_point(&mut self, v: f64) {
        self.points[self.point_count & POINT_MASK] = v.abs();
        self.point_count = self.point_count.wrapping_add(1);
    }

    /// The 4× point `back` steps before the newest one.
    fn point(&self, back: usize) -> f64 {
        self.points[self.point_count.wrapping_sub(1 + back) & POINT_MASK]
    }

    /// Pushes input sample `u[n]` and returns the peak estimate `q ≥ 0` of the interval
    /// between `u[n − 17]` and `u[n − 16]`. RT-safe.
    #[inline]
    pub fn push(&mut self, u: f64) -> f64 {
        let pos = self.hist_pos;
        self.hist[pos] = u;
        self.hist[pos + TAPS_PER_PHASE] = u;
        self.hist_pos = if pos + 1 == TAPS_PER_PHASE {
            0
        } else {
            pos + 1
        };
        // The 32 most recent inputs, oldest first: w[i] = u[n − 31 + i].
        let w = &self.hist[pos + 1..pos + 1 + TAPS_PER_PHASE];
        let centre = w[DETECTOR_DELAY - 1]; // u[n − 16]
        let mut frac = [0.0; OVERSAMPLING - 1];
        for (v, taps) in frac.iter_mut().zip(&self.coeffs) {
            *v = taps.iter().zip(w).map(|(c, x)| c * x).sum();
        }
        self.push_point(centre);
        for v in frac {
            self.push_point(v);
        }
        // Newest point: v(n − 16 + 3/4). The interval (n − 17, n − 16) spans points 7 (u[n − 17])
        // … 3 (u[n − 16]) steps back; points 8 and 2 are the outer neighbours.
        let mut q = 0.0_f64;
        for back in 3..=7 {
            let (a, b, c) = (self.point(back + 1), self.point(back), self.point(back - 1));
            q = q.max(b);
            if b >= a && b >= c {
                let curvature = a - 2.0 * b + c;
                if curvature < 0.0 {
                    // Vertex of the parabola through (−1, a), (0, b), (1, c); a local maximum puts
                    // it within ±½ step.
                    q = q.max(b - (a - c) * (a - c) / (8.0 * curvature));
                }
            }
        }
        q
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn to_db(x: f64) -> f64 {
        20.0 * x.log10()
    }

    #[test]
    fn phases_have_unity_dc_gain_and_are_mirror_images() {
        let c = phase_coefficients();
        for taps in &c {
            let dc: f64 = taps.iter().sum();
            assert!((dc - 1.0).abs() < 1e-12);
        }
        for i in 0..TAPS_PER_PHASE {
            assert!((c[0][i] - c[2][TAPS_PER_PHASE - 1 - i]).abs() < 1e-15);
            assert!((c[1][i] - c[1][TAPS_PER_PHASE - 1 - i]).abs() < 1e-15);
        }
    }

    #[test]
    fn delay_is_sixteen_samples_and_sample_peaks_are_exact() {
        // A lone sample: every interval touching it reads exactly its magnitude.
        let mut d = TruePeakDetector::new();
        let mut q = Vec::new();
        for n in 0..64 {
            q.push(d.push(if n == 10 { -0.75 } else { 0.0 }));
        }
        // Interval (n − 17, n − 16) contains sample 10 for n = 26 (ends at 10) and n = 27.
        assert!(q[25] < 0.75, "{}", q[25]);
        assert!((q[26] - 0.75).abs() < 1e-15, "{}", q[26]);
        assert!((q[27] - 0.75).abs() < 1e-15, "{}", q[27]);
        assert!(q[28] < 0.75);
    }

    #[test]
    fn dc_reads_exactly() {
        let mut d = TruePeakDetector::new();
        let mut last = 0.0;
        for _ in 0..100 {
            last = d.push(0.5);
        }
        assert!((last - 0.5).abs() < 1e-12, "{last}");
    }

    /// 4× + parabola reads steady sines up to 20 kHz within a few hundredths of a dB.
    #[test]
    fn sine_peaks_read_close_to_analytic() {
        for &rate in &[44_100.0, 48_000.0, 96_000.0] {
            let mut worst = (0.0_f64, 0.0_f64);
            for &f in &[997.0, 5_000.0, 10_000.0, 15_000.0, 18_000.0, 19_900.0] {
                if f > 0.4535 * rate {
                    continue;
                }
                for k in 0..8 {
                    let phase = f64::from(k) * 0.37;
                    let w = std::f64::consts::TAU * f / rate;
                    let mut d = TruePeakDetector::new();
                    let mut max: f64 = 0.0;
                    for n in 0..4_000 {
                        let q = d.push((w * f64::from(n) + phase).sin());
                        if n > 200 {
                            max = max.max(q);
                        }
                    }
                    let err = to_db(max);
                    worst = (worst.0.min(err), worst.1.max(err));
                }
            }
            assert!(
                worst.0 > -0.05 && worst.1 < 0.05,
                "{rate} Hz: error {worst:?} dB"
            );
        }
    }
}

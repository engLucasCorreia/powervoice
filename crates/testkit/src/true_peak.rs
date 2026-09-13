//! Normative true-peak meter for tests (SPEC-017 §4.6).
//!
//! - [`true_peak_reference_dbtp`]: 16× polyphase Kaiser-windowed sinc, 128 taps per phase
//!   (half-width 64 input samples), β = 15.6 (≈ 150 dB), every fractional phase normalized to
//!   unity DC gain, plus parabolic refinement at every local maximum of |v|. Input is
//!   zero-padded; the scan covers the reconstruction 64 samples beyond both ends.
//! - [`true_peak_reference_range_dbtp`]: the same over the intervals of a sample range (the
//!   whole signal is still used as interpolation context) — steady-state and accuracy checks.
//! - [`true_peak_4x_dbtp`]: the plain 4× points (phases 0, 4, 8, 12) of the same interpolator,
//!   no refinement — PROMPT §5's literal "4×-oversampled peak".
//!
//! `ebur128`'s true peak (see [`crate::measure::loudness`]) is only a cross-check: it reads
//! −0.20 … +0.20 dB off depending on content (MEMORY, SPEC-017 §4.6).
//!
//! Conventions as in [`crate::measure`]: `-inf` for digital silence, `NaN` for non-finite input.

use std::ops::Range;

use crate::bandlimit::{kaiser, sinc};
use crate::units::linear_to_dbfs;

/// Oversampling factor of the reference.
pub const REFERENCE_OVERSAMPLING: usize = 16;
/// Taps per polyphase branch.
pub const REFERENCE_TAPS_PER_PHASE: usize = 128;
/// Kaiser β of the reference interpolator.
pub const REFERENCE_BETA: f64 = 15.6;

const HALF: usize = REFERENCE_TAPS_PER_PHASE / 2;
/// Zero padding on each side of the signal (≥ 2 · HALF so the extended scan stays in bounds).
const PAD: usize = 2 * HALF;

/// Coefficients of phase `p/16` (`p = 1..16`): applied to `x[m − 63 ..= m + 64]` they give the
/// interpolated value at `m + p/16`. Index 0 is unused (phase 0 is the sample itself).
fn coefficients() -> Vec<[f64; REFERENCE_TAPS_PER_PHASE]> {
    (0..REFERENCE_OVERSAMPLING)
        .map(|p| {
            let mut taps = [0.0; REFERENCE_TAPS_PER_PHASE];
            if p == 0 {
                return taps;
            }
            let frac = p as f64 / REFERENCE_OVERSAMPLING as f64;
            for (i, c) in taps.iter_mut().enumerate() {
                let tau = frac + (HALF as f64 - 1.0) - i as f64;
                *c = sinc(tau) * kaiser(tau / HALF as f64, REFERENCE_BETA);
            }
            let dc: f64 = taps.iter().sum();
            for c in &mut taps {
                *c /= dc;
            }
            taps
        })
        .collect()
}

/// Running max over a stream of |v| points with optional parabolic refinement.
struct Scanner {
    refine: bool,
    prev: [f64; 2],
    prev_counted: bool,
    seen: usize,
    max: f64,
}

impl Scanner {
    fn feed(&mut self, v: f64, counted: bool) {
        // Judge the previous point now that both of its neighbours are known.
        if self.seen >= 2 && self.prev_counted {
            let (a, b, c) = (self.prev[0], self.prev[1], v);
            self.max = self.max.max(b);
            if self.refine && b >= a && b >= c {
                let curvature = a - 2.0 * b + c;
                if curvature < 0.0 {
                    self.max = self.max.max(b - (a - c) * (a - c) / (8.0 * curvature));
                }
            }
        }
        self.prev = [self.prev[1], v];
        self.prev_counted = counted;
        self.seen += 1;
    }
}

/// Linear peak of the reconstruction over the 4×/16× points of intervals starting at original
/// indices `range` (clamped to `[−64, n + 64)`), stepping `16 / oversampling` phases.
fn scan(samples: &[f32], range: Range<i64>, oversampling: usize, refine: bool) -> f64 {
    let n = samples.len() as i64;
    let start = range.start.max(-(HALF as i64));
    let end = range.end.min(n + HALF as i64);
    if start >= end {
        return 0.0;
    }
    let mut xp = vec![0.0f64; samples.len() + 2 * PAD];
    for (dst, &s) in xp[PAD..].iter_mut().zip(samples) {
        *dst = f64::from(s);
    }
    let coeffs = coefficients();
    let step = REFERENCE_OVERSAMPLING / oversampling;
    let value = |m: usize, p: usize| -> f64 {
        if p == 0 {
            xp[m].abs()
        } else {
            let w = &xp[m + 1 - HALF..=m + HALF];
            coeffs[p]
                .iter()
                .zip(w)
                .map(|(c, x)| c * x)
                .sum::<f64>()
                .abs()
        }
    };
    let mut s = Scanner {
        refine,
        prev: [0.0; 2],
        prev_counted: false,
        seen: 0,
        max: 0.0,
    };
    // Padded index of original sample i is i + PAD.
    let first = (start + PAD as i64) as usize;
    let last = (end + PAD as i64) as usize; // exclusive
    // Left neighbour of the first point.
    s.feed(value(first - 1, REFERENCE_OVERSAMPLING - step), false);
    for m in first..last {
        for p in (0..REFERENCE_OVERSAMPLING).step_by(step) {
            s.feed(value(m, p), true);
        }
    }
    // Right neighbour of the last point, then flush it.
    s.feed(value(last, 0), false);
    s.feed(0.0, false);
    s.max
}

fn to_dbtp(samples: &[f32], peak: impl FnOnce() -> f64) -> f64 {
    if samples.iter().any(|x| !x.is_finite()) {
        return f64::NAN;
    }
    linear_to_dbfs(peak())
}

/// Reference true peak (dBTP) of a mono signal: 16× oversampling + parabolic refinement.
/// Accuracy ≤ 0.005 dB on sines up to 0.45·fs (SPEC-017 AC-5).
pub fn true_peak_reference_dbtp(samples: &[f32]) -> f64 {
    let n = samples.len() as i64;
    to_dbtp(samples, || {
        scan(samples, -(HALF as i64)..n + HALF as i64, 16, true)
    })
}

/// Reference true peak (dBTP) of the intervals starting at the samples in `range` (the whole
/// signal is interpolation context).
pub fn true_peak_reference_range_dbtp(samples: &[f32], range: Range<usize>) -> f64 {
    to_dbtp(samples, || {
        scan(samples, range.start as i64..range.end as i64, 16, true)
    })
}

/// Plain 4× points of the reference interpolator (no refinement), dBTP.
pub fn true_peak_4x_dbtp(samples: &[f32]) -> f64 {
    let n = samples.len() as i64;
    to_dbtp(samples, || {
        scan(samples, -(HALF as i64)..n + HALF as i64, 4, false)
    })
}

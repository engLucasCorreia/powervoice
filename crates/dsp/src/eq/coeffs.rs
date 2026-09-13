//! EQ section coefficients (SPEC-015 §4.2), all f64 and normalized by `a0`.
//!
//! RBJ Audio EQ Cookbook (W3C note): A = 10^(gain_dB/40), ω0 = 2π·f/fs with f clamped to
//! 0.49·fs, α = sin ω0 / (2Q). Butterworth HP/LP of order N = ⌊N/2⌋ cookbook HPF/LPF biquads at
//! the same f with the Butterworth Qs, plus one bilinear first-order section (K = tan(π f/fs))
//! for odd N; every section shares the prewarp frequency, so the cascade is exactly the bilinear
//! transform of the analog prototype (|H(fc)| = −3.0103 dB for every N).

use std::f64::consts::PI;

use super::{MAX_ORDER, MAX_SECTIONS, clamp_freq_hz};

/// One section `H(z) = (b0 + b1 z⁻¹ + b2 z⁻²) / (1 + a1 z⁻¹ + a2 z⁻²)`. First-order sections
/// have `b2 = a2 = 0`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Biquad {
    /// Feed-forward 0.
    pub b0: f64,
    /// Feed-forward 1.
    pub b1: f64,
    /// Feed-forward 2.
    pub b2: f64,
    /// Feedback 1.
    pub a1: f64,
    /// Feedback 2.
    pub a2: f64,
}

/// `cos ω0`, `sin ω0` at the clamped frequency.
fn trig(freq_hz: f64, sample_rate: f64) -> (f64, f64) {
    let w0 = 2.0 * PI * clamp_freq_hz(freq_hz, sample_rate) / sample_rate;
    (w0.cos(), w0.sin())
}

impl Biquad {
    /// `y = x`.
    pub const IDENTITY: Self = Self {
        b0: 1.0,
        b1: 0.0,
        b2: 0.0,
        a1: 0.0,
        a2: 0.0,
    };

    fn normalized(b0: f64, b1: f64, b2: f64, a0: f64, a1: f64, a2: f64) -> Self {
        Self {
            b0: b0 / a0,
            b1: b1 / a0,
            b2: b2 / a0,
            a1: a1 / a0,
            a2: a2 / a0,
        }
    }

    /// Cookbook `peakingEQ`: |H(f)| = gain exactly; a cut is the exact inverse of the boost.
    pub fn peaking(freq_hz: f64, q: f64, gain_db: f64, sample_rate: f64) -> Self {
        let (cos, sin) = trig(freq_hz, sample_rate);
        let a = 10f64.powf(gain_db / 40.0);
        let alpha = sin / (2.0 * q);
        Self::normalized(
            1.0 + alpha * a,
            -2.0 * cos,
            1.0 - alpha * a,
            1.0 + alpha / a,
            -2.0 * cos,
            1.0 - alpha / a,
        )
    }

    /// Cookbook `lowShelf` (Q form): gain below, 0 dB above, gain/2 at the midpoint `freq_hz`.
    pub fn low_shelf(freq_hz: f64, q: f64, gain_db: f64, sample_rate: f64) -> Self {
        let (cos, sin) = trig(freq_hz, sample_rate);
        let a = 10f64.powf(gain_db / 40.0);
        let s = 2.0 * a.sqrt() * sin / (2.0 * q);
        let (ap, am) = (a + 1.0, a - 1.0);
        Self::normalized(
            a * (ap - am * cos + s),
            2.0 * a * (am - ap * cos),
            a * (ap - am * cos - s),
            ap + am * cos + s,
            -2.0 * (am + ap * cos),
            ap + am * cos - s,
        )
    }

    /// Cookbook `highShelf` (Q form): gain above, 0 dB below, gain/2 at the midpoint `freq_hz`.
    pub fn high_shelf(freq_hz: f64, q: f64, gain_db: f64, sample_rate: f64) -> Self {
        let (cos, sin) = trig(freq_hz, sample_rate);
        let a = 10f64.powf(gain_db / 40.0);
        let s = 2.0 * a.sqrt() * sin / (2.0 * q);
        let (ap, am) = (a + 1.0, a - 1.0);
        Self::normalized(
            a * (ap + am * cos + s),
            -2.0 * a * (am + ap * cos),
            a * (ap + am * cos - s),
            ap - am * cos + s,
            2.0 * (am - ap * cos),
            ap - am * cos - s,
        )
    }

    /// Cookbook `LPF` from precomputed `cos ω0`, `sin ω0`.
    fn lowpass_trig(cos: f64, sin: f64, q: f64) -> Self {
        let alpha = sin / (2.0 * q);
        let c = 1.0 - cos;
        Self::normalized(c / 2.0, c, c / 2.0, 1.0 + alpha, -2.0 * cos, 1.0 - alpha)
    }

    /// Cookbook `HPF` from precomputed `cos ω0`, `sin ω0`.
    fn highpass_trig(cos: f64, sin: f64, q: f64) -> Self {
        let alpha = sin / (2.0 * q);
        let c = 1.0 + cos;
        Self::normalized(c / 2.0, -c, c / 2.0, 1.0 + alpha, -2.0 * cos, 1.0 - alpha)
    }

    /// Cookbook `LPF`.
    pub fn lowpass(freq_hz: f64, q: f64, sample_rate: f64) -> Self {
        let (cos, sin) = trig(freq_hz, sample_rate);
        Self::lowpass_trig(cos, sin, q)
    }

    /// Cookbook `HPF`.
    pub fn highpass(freq_hz: f64, q: f64, sample_rate: f64) -> Self {
        let (cos, sin) = trig(freq_hz, sample_rate);
        Self::highpass_trig(cos, sin, q)
    }

    /// First-order bilinear low-pass: b = [K, K]/(1+K), a1 = (K−1)/(K+1), K = tan(π f/fs).
    pub fn lowpass1(freq_hz: f64, sample_rate: f64) -> Self {
        let k = (PI * clamp_freq_hz(freq_hz, sample_rate) / sample_rate).tan();
        Self {
            b0: k / (1.0 + k),
            b1: k / (1.0 + k),
            b2: 0.0,
            a1: (k - 1.0) / (k + 1.0),
            a2: 0.0,
        }
    }

    /// First-order bilinear high-pass: b = [1, −1]/(1+K), a1 = (K−1)/(K+1).
    pub fn highpass1(freq_hz: f64, sample_rate: f64) -> Self {
        let k = (PI * clamp_freq_hz(freq_hz, sample_rate) / sample_rate).tan();
        Self {
            b0: 1.0 / (1.0 + k),
            b1: -1.0 / (1.0 + k),
            b2: 0.0,
            a1: (k - 1.0) / (k + 1.0),
            a2: 0.0,
        }
    }

    /// |H|² at `freq_hz` (cookbook robust form, φ = sin²(ω/2)); ≥ 0.
    pub fn magnitude_sq(&self, freq_hz: f64, sample_rate: f64) -> f64 {
        let phi = (PI * freq_hz / sample_rate).sin().powi(2);
        let Self { b0, b1, b2, a1, a2 } = *self;
        let num = (b0 + b1 + b2).powi(2) - 4.0 * (b0 * b1 + 4.0 * b0 * b2 + b1 * b2) * phi
            + 16.0 * b0 * b2 * phi * phi;
        let den =
            (1.0 + a1 + a2).powi(2) - 4.0 * (a1 + 4.0 * a2 + a1 * a2) * phi + 16.0 * a2 * phi * phi;
        (num / den).max(0.0)
    }

    /// |H| in dB at `freq_hz` (floored at −3000 dB for exact zeros).
    pub fn magnitude_db(&self, freq_hz: f64, sample_rate: f64) -> f64 {
        10.0 * self.magnitude_sq(freq_hz, sample_rate).max(1e-300).log10()
    }

    /// Largest pole magnitude (1.0 or more means unstable).
    pub fn pole_radius(&self) -> f64 {
        let disc = self.a1 * self.a1 - 4.0 * self.a2;
        if disc < 0.0 {
            self.a2.sqrt()
        } else {
            (self.a1.abs() + disc.sqrt()) / 2.0
        }
    }
}

/// The three shelf/peak band shapes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GainShape {
    /// [`Biquad::low_shelf`].
    LowShelf,
    /// [`Biquad::peaking`].
    Peak,
    /// [`Biquad::high_shelf`].
    HighShelf,
}

impl GainShape {
    /// The section for these values.
    pub fn coeffs(self, freq_hz: f64, q: f64, gain_db: f64, sample_rate: f64) -> Biquad {
        match self {
            Self::LowShelf => Biquad::low_shelf(freq_hz, q, gain_db, sample_rate),
            Self::Peak => Biquad::peaking(freq_hz, q, gain_db, sample_rate),
            Self::HighShelf => Biquad::high_shelf(freq_hz, q, gain_db, sample_rate),
        }
    }
}

/// Butterworth high-pass or low-pass.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PassKind {
    /// High-pass.
    HighPass,
    /// Low-pass.
    LowPass,
}

/// Butterworth biquad Q number `k` (1-based) of order `order` (SPEC-015 §4.2):
/// odd N: 1/(2 cos(kπ/N)); even N: 1/(2 cos((2k−1)π/(2N))).
pub fn butterworth_q(order: usize, k: usize) -> f64 {
    let n = order as f64;
    let k = k as f64;
    let theta = if order % 2 == 1 {
        k * PI / n
    } else {
        (2.0 * k - 1.0) * PI / (2.0 * n)
    };
    1.0 / (2.0 * theta.cos())
}

/// A cascade's sections: `coeffs[..len]` in processing order.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sections {
    /// Section coefficients (unused entries are [`Biquad::IDENTITY`]).
    pub coeffs: [Biquad; MAX_SECTIONS],
    /// Sections in use.
    pub len: usize,
}

impl Sections {
    /// The cascade's |H| in dB at `freq_hz`.
    pub fn magnitude_db(&self, freq_hz: f64, sample_rate: f64) -> f64 {
        self.coeffs[..self.len]
            .iter()
            .map(|c| c.magnitude_db(freq_hz, sample_rate))
            .sum()
    }

    /// Largest pole magnitude of the cascade.
    pub fn pole_radius(&self) -> f64 {
        self.coeffs[..self.len]
            .iter()
            .fold(0.0, |r, c| r.max(c.pole_radius()))
    }
}

/// Butterworth cascade of order `order` (clamped to 1…8) at `freq_hz`: the first-order section
/// first (odd N), then the biquads in ascending Q.
pub fn butterworth(kind: PassKind, order: usize, freq_hz: f64, sample_rate: f64) -> Sections {
    let order = order.clamp(1, MAX_ORDER);
    let mut s = Sections {
        coeffs: [Biquad::IDENTITY; MAX_SECTIONS],
        len: 0,
    };
    if order % 2 == 1 {
        s.coeffs[0] = match kind {
            PassKind::HighPass => Biquad::highpass1(freq_hz, sample_rate),
            PassKind::LowPass => Biquad::lowpass1(freq_hz, sample_rate),
        };
        s.len = 1;
    }
    let (cos, sin) = trig(freq_hz, sample_rate);
    for k in 1..=order / 2 {
        let q = butterworth_q(order, k);
        s.coeffs[s.len] = match kind {
            PassKind::HighPass => Biquad::highpass_trig(cos, sin, q),
            PassKind::LowPass => Biquad::lowpass_trig(cos, sin, q),
        };
        s.len += 1;
    }
    s
}

#[cfg(test)]
#[allow(clippy::approx_constant)] // SPEC-015 table values (0.7071), not 1/√2
mod tests {
    use super::*;

    const FS: f64 = 48_000.0;

    #[test]
    fn butterworth_qs_match_the_spec_table() {
        let table: [&[f64]; 8] = [
            &[],
            &[0.7071],
            &[1.0],
            &[0.5412, 1.3066],
            &[0.6180, 1.6180],
            &[0.5176, 0.7071, 1.9319],
            &[0.5550, 0.8019, 2.2470],
            &[0.5098, 0.6013, 0.9000, 2.5629],
        ];
        for (i, qs) in table.iter().enumerate() {
            let order = i + 1;
            let s = butterworth(PassKind::LowPass, order, 1000.0, FS);
            assert_eq!(s.len, order.div_ceil(2), "N {order}");
            for (k, &q) in qs.iter().enumerate() {
                let got = butterworth_q(order, k + 1);
                assert!(
                    (got - q).abs() < 1e-4,
                    "N {order} k {}: {got} vs {q}",
                    k + 1
                );
            }
        }
    }

    #[test]
    fn butterworth_is_minus_3db_at_fc_for_every_order() {
        for fs in [44_100.0, 48_000.0, 96_000.0] {
            for order in 1..=MAX_ORDER {
                for fc in [30.0, 80.0, 1_000.0, 8_000.0] {
                    for kind in [PassKind::HighPass, PassKind::LowPass] {
                        let db = butterworth(kind, order, fc, fs).magnitude_db(fc, fs);
                        assert!(
                            (db + 3.010_3).abs() < 1e-3,
                            "{kind:?} N {order} {fc} Hz: {db}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn peak_and_shelf_semantics() {
        for g in [-24.0, -12.0, -3.0, 3.0, 12.0, 24.0] {
            for q in [0.1, 1.0, 30.0] {
                let p = Biquad::peaking(1000.0, q, g, FS);
                assert!(
                    (p.magnitude_db(1000.0, FS) - g).abs() < 0.01,
                    "peak {g} Q {q}"
                );
                // Boost followed by the equal cut is flat.
                let c = Biquad::peaking(1000.0, q, -g, FS);
                for f in [20.0, 200.0, 999.0, 5000.0, 20_000.0] {
                    let sum = p.magnitude_db(f, FS) + c.magnitude_db(f, FS);
                    assert!(sum.abs() < 1e-6, "peak ±{g} Q {q} at {f}: {sum}");
                }
            }
            for q in [0.3, 0.7071, 2.0] {
                let ls = Biquad::low_shelf(1000.0, q, g, FS);
                let hs = Biquad::high_shelf(1000.0, q, g, FS);
                assert!((ls.magnitude_db(1000.0, FS) - g / 2.0).abs() < 0.01);
                assert!((hs.magnitude_db(1000.0, FS) - g / 2.0).abs() < 0.01);
            }
            let ls = Biquad::low_shelf(400.0, 0.7071, g, FS);
            assert!(
                (ls.magnitude_db(20.0, FS) - g).abs() < 0.05,
                "ls {g} at 20 Hz"
            );
            assert!(
                ls.magnitude_db(20_000.0, FS).abs() < 0.05,
                "ls {g} far side"
            );
            let hs = Biquad::high_shelf(1000.0, 0.7071, g, FS);
            assert!(
                (hs.magnitude_db(20_000.0, FS) - g).abs() < 0.05,
                "hs {g} at 20 kHz"
            );
            assert!(hs.magnitude_db(20.0, FS).abs() < 0.05, "hs {g} far side");
        }
    }

    #[test]
    fn frequency_is_clamped_to_049_fs() {
        let fs = 22_050.0;
        let top = 0.49 * fs;
        assert_eq!(
            Biquad::peaking(20_000.0, 4.0, 6.0, fs),
            Biquad::peaking(top, 4.0, 6.0, fs)
        );
        assert_eq!(
            butterworth(PassKind::HighPass, 7, 20_000.0, fs),
            butterworth(PassKind::HighPass, 7, top, fs)
        );
    }

    #[test]
    fn every_reachable_section_is_stable() {
        for fs in [8_000.0, 44_100.0, 96_000.0] {
            for f in [20.0, 20_000.0] {
                for q in [0.1, 30.0] {
                    for g in [-24.0, 24.0] {
                        for shape in [GainShape::LowShelf, GainShape::Peak, GainShape::HighShelf] {
                            let r = shape.coeffs(f, q, g, fs).pole_radius();
                            assert!(r < 1.0, "{shape:?} {f} Q {q} {g} dB @ {fs}: r {r}");
                        }
                    }
                }
                for order in 1..=MAX_ORDER {
                    for kind in [PassKind::HighPass, PassKind::LowPass] {
                        let r = butterworth(kind, order, f, fs).pole_radius();
                        assert!(r < 1.0, "{kind:?} N {order} {f} @ {fs}: r {r}");
                    }
                }
            }
        }
    }
}

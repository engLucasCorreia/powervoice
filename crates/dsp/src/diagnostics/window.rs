//! Analysis windows for the Spectrum Inspector and the long-term average job (H-42, SPEC-007
//! §8.3). All windows are **periodic** (DFT-even: the `N + 1`-point symmetric window with its
//! last point dropped), the usual choice for spectral analysis.
//!
//! Every window is a cosine sum `w[n] = Σ_k (−1)^k a_k cos(2πkn/N)`:
//! - **Hann** `[0.5, 0.5]` — ENBW 1.5 bins, the Inspector's and the LTAS job's default;
//! - **Blackman-Harris 4-term** — SPEC-012 §4.3 coefficients, the live analyzer's window
//!   (ENBW 2.004 bins, −92 dB sidelobes);
//! - **Flat-top** — the SR785 / MATLAB `flattopwin` coefficients (ENBW 3.77 bins, < 0.02 dB
//!   scalloping: the one to read a tone's level off);
//! - **Rectangular** `[1]` — ENBW 1 bin, the finest resolution, −13 dB sidelobes.

/// A spectral-analysis window (wire codes 0 … 3, SPEC-007 §8.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum WindowKind {
    #[default]
    Hann,
    BlackmanHarris,
    FlatTop,
    Rectangular,
}

const HANN: [f64; 2] = [0.5, 0.5];
const BLACKMAN_HARRIS: [f64; 4] = [0.35875, 0.48829, 0.14128, 0.01168];
const FLAT_TOP: [f64; 5] = [
    0.215_578_95,
    0.416_631_58,
    0.277_263_158,
    0.083_578_947,
    0.006_947_368,
];
const RECTANGULAR: [f64; 1] = [1.0];

impl WindowKind {
    /// Every window, in wire-code order.
    pub const ALL: [WindowKind; 4] = [
        WindowKind::Hann,
        WindowKind::BlackmanHarris,
        WindowKind::FlatTop,
        WindowKind::Rectangular,
    ];

    /// Wire code (0 Hann, 1 Blackman-Harris, 2 flat-top, 3 rectangular).
    pub fn code(self) -> u32 {
        match self {
            WindowKind::Hann => 0,
            WindowKind::BlackmanHarris => 1,
            WindowKind::FlatTop => 2,
            WindowKind::Rectangular => 3,
        }
    }

    pub fn from_code(code: u32) -> Option<Self> {
        Self::ALL.get(code as usize).copied()
    }

    fn cosine_terms(self) -> &'static [f64] {
        match self {
            WindowKind::Hann => &HANN,
            WindowKind::BlackmanHarris => &BLACKMAN_HARRIS,
            WindowKind::FlatTop => &FLAT_TOP,
            WindowKind::Rectangular => &RECTANGULAR,
        }
    }

    /// The periodic window of length `n`.
    pub fn coefficients(self, n: usize) -> Vec<f64> {
        let terms = self.cosine_terms();
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
}

/// Equivalent noise bandwidth in bins, `N·Σw² / (Σw)²`: the factor that turns a sum of
/// sine-normalized bin powers into a band power (SPEC-007 §8.4).
pub fn enbw_bins(window: &[f64]) -> f64 {
    let sum: f64 = window.iter().sum();
    let sum_sq: f64 = window.iter().map(|w| w * w).sum();
    if sum == 0.0 {
        return 1.0;
    }
    window.len() as f64 * sum_sq / (sum * sum)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enbw_matches_the_textbook_values() {
        // Harris 1978 / Heinzel, Rüdiger & Schilling 2002 (periodic windows, large N).
        let n = 8192;
        let cases = [
            (WindowKind::Hann, 1.5),
            (WindowKind::BlackmanHarris, 2.0044),
            (WindowKind::FlatTop, 3.7702),
            (WindowKind::Rectangular, 1.0),
        ];
        for (kind, expected) in cases {
            let enbw = enbw_bins(&kind.coefficients(n));
            assert!(
                (enbw - expected).abs() < 2e-3,
                "{kind:?}: ENBW {enbw} vs {expected}"
            );
        }
    }

    #[test]
    fn windows_are_periodic_and_peak_in_the_middle() {
        for kind in WindowKind::ALL {
            let w = kind.coefficients(16);
            assert_eq!(w.len(), 16);
            // Periodic: symmetric about n = N/2 (w[k] == w[N−k]).
            for k in 1..8 {
                assert!((w[k] - w[16 - k]).abs() < 1e-12, "{kind:?} not periodic");
            }
            let max = w.iter().cloned().fold(f64::MIN, f64::max);
            assert!((w[8] - max).abs() < 1e-12, "{kind:?} doesn't peak at N/2");
        }
        assert!(WindowKind::Hann.coefficients(8)[0].abs() < 1e-12);
    }

    #[test]
    fn codes_round_trip() {
        for kind in WindowKind::ALL {
            assert_eq!(WindowKind::from_code(kind.code()), Some(kind));
        }
        assert_eq!(WindowKind::from_code(4), None);
    }
}

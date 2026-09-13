//! Static gain computers in dB (SPEC-016 §4.4; Giannoulis, Massberg & Reiss 2012, eq. 4 and its
//! mirror). The single source for `process()` and the future `TransferCurve`.

use super::GAIN_FLOOR_DB;

/// Knee widths below this are a hard knee.
const HARD_KNEE_DB: f64 = 1e-9;

/// Compressor slope `1 − 1/R` (the ramped domain of the ratio, §4.8).
pub fn compressor_slope(ratio: f64) -> f64 {
    1.0 - 1.0 / ratio.max(1.0)
}

/// Compressor gain (dB, `<= 0`) for input level `x`, threshold `t`, slope `1 − 1/R` and knee
/// width `w` (dB). Clamped to [`GAIN_FLOOR_DB`].
pub fn compressor_gain_db(x: f64, t: f64, slope: f64, w: f64) -> f64 {
    let d = x - t;
    let g = if w < HARD_KNEE_DB {
        if d > 0.0 { -slope * d } else { 0.0 }
    } else if 2.0 * d < -w {
        0.0
    } else if 2.0 * d.abs() <= w {
        let k = d + w / 2.0;
        -slope * k * k / (2.0 * w)
    } else {
        -slope * d
    };
    g.max(GAIN_FLOOR_DB)
}

/// Downward-expander gain (dB, `<= 0`) for input level `x`, threshold `t`, `R − 1` and knee
/// width `w` (dB). Clamped to [`GAIN_FLOOR_DB`].
pub fn expander_gain_db(x: f64, t: f64, ratio_minus_one: f64, w: f64) -> f64 {
    let d = x - t;
    let g = if w < HARD_KNEE_DB {
        if d < 0.0 { ratio_minus_one * d } else { 0.0 }
    } else if 2.0 * d > w {
        0.0
    } else if 2.0 * d.abs() <= w {
        let k = d - w / 2.0;
        -ratio_minus_one * k * k / (2.0 * w)
    } else {
        ratio_minus_one * d
    };
    g.max(GAIN_FLOOR_DB)
}

/// Limiter gain `min(0, t − x)` (hard, never a knee), clamped to [`GAIN_FLOOR_DB`].
pub fn limiter_gain_db(x: f64, t: f64) -> f64 {
    (t - x).clamp(GAIN_FLOOR_DB, 0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-12
    }

    #[test]
    fn compressor_hard_and_soft_knee() {
        let s = compressor_slope(4.0);
        assert!(close(s, 0.75));
        assert!(close(compressor_gain_db(-30.0, -20.0, s, 0.0), 0.0));
        assert!(close(compressor_gain_db(-10.0, -20.0, s, 0.0), -7.5));
        // Soft knee: 0 below T − W/2, straight line above T + W/2, continuous at both edges.
        assert!(close(compressor_gain_db(-23.0, -20.0, s, 6.0), 0.0));
        assert!(close(compressor_gain_db(-17.0, -20.0, s, 6.0), -0.75 * 3.0));
        assert!(close(
            compressor_gain_db(-20.0, -20.0, s, 6.0),
            -0.75 * 9.0 / 12.0
        ));
        for edge in [-23.0, -17.0] {
            let a = compressor_gain_db(edge - 1e-9, -20.0, s, 6.0);
            let b = compressor_gain_db(edge + 1e-9, -20.0, s, 6.0);
            assert!((a - b).abs() < 1e-8, "discontinuous at {edge}");
        }
        assert!(close(
            compressor_gain_db(200.0, -60.0, 1.0, 0.0),
            GAIN_FLOOR_DB
        ));
        assert!(close(compressor_slope(1.0), 0.0));
    }

    #[test]
    fn expander_mirror() {
        assert!(close(expander_gain_db(-30.0, -40.0, 1.0, 0.0), 0.0));
        assert!(close(expander_gain_db(-50.0, -40.0, 1.0, 0.0), -10.0));
        assert!(close(expander_gain_db(-37.0, -40.0, 1.0, 6.0), 0.0));
        assert!(close(expander_gain_db(-43.0, -40.0, 1.0, 6.0), -3.0));
        assert!(close(expander_gain_db(-40.0, -40.0, 1.0, 6.0), -9.0 / 12.0));
        assert!(close(
            expander_gain_db(-150.0, -20.0, 29.0, 0.0),
            GAIN_FLOOR_DB
        ));
    }

    #[test]
    fn limiter() {
        assert!(close(limiter_gain_db(-12.0, -10.0), 0.0));
        assert!(close(limiter_gain_db(-4.0, -10.0), -6.0));
        assert!(close(limiter_gain_db(500.0, -30.0), GAIN_FLOOR_DB));
    }
}

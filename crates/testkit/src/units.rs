//! dB <-> linear amplitude helpers (full-scale = 1.0).

/// Convert a level in dBFS to a linear amplitude (`1.0` == 0 dBFS).
pub fn dbfs_to_linear(db: f64) -> f64 {
    10f64.powf(db / 20.0)
}

/// Convert a linear amplitude to dBFS.
///
/// This is deliberately a bare `20*log10`, relying on IEEE 754 semantics
/// rather than an artificial floor: exact digital silence (`0.0`) maps to
/// `-inf` (the correct dB value for zero amplitude, and the project-wide
/// convention for silence -- see `measure` module docs), and `NaN` in
/// propagates to `NaN` out. A previous version clamped small magnitudes to a
/// floor, which had the side effect of turning `NaN` (e.g. from a
/// divide-by-zero upstream) into a finite, plausible-looking `-240 dBFS`
/// instead of surfacing the problem -- see `measure::peak_dbfs` etc. for the
/// explicit non-finite-input checks that now guard against that.
pub fn linear_to_dbfs(linear: f64) -> f64 {
    20.0 * linear.abs().log10()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips() {
        for db in [-60.0, -23.0, -20.0, -6.0, -0.1, 0.0] {
            let linear = dbfs_to_linear(db);
            assert!((linear_to_dbfs(linear) - db).abs() < 1e-9);
        }
    }

    #[test]
    fn zero_dbfs_is_unity() {
        assert!((dbfs_to_linear(0.0) - 1.0).abs() < 1e-12);
    }

    #[test]
    #[allow(clippy::float_cmp)] // deliberate exact-value check, not a tolerance comparison
    fn exact_silence_is_negative_infinity() {
        assert_eq!(linear_to_dbfs(0.0), f64::NEG_INFINITY);
    }

    #[test]
    fn nan_propagates() {
        assert!(linear_to_dbfs(f64::NAN).is_nan());
    }
}

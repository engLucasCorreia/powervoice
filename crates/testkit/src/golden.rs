//! Golden helpers: compare buffers with tolerance.

/// FNV-1a hash over a buffer's raw bits (`f32::to_bits`), for pinning
/// "golden" generator output across changes.
///
/// Deliberately not `std::hash::DefaultHasher` (SipHash): its output is
/// explicitly *not* guaranteed stable across Rust releases, which makes it
/// unusable as a committed golden constant. FNV-1a is a fixed, simple
/// algorithm with no such caveat. Hashing raw bits (not the `f32` values
/// through a float-aware combinator) also avoids `powf`/`sin`/`log10` ULP
/// differences across platforms leaking into the hash -- callers should
/// still generate the reference signal in a way that avoids relying on
/// transcendental-function ULP precision (e.g. prefer PRNG-only signals
/// such as white/pink noise over `sin`-based tones for golden hashes).
pub fn fnv1a_hash(samples: &[f32]) -> u64 {
    const OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut hash = OFFSET_BASIS;
    for &x in samples {
        for byte in x.to_bits().to_le_bytes() {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(PRIME);
        }
    }
    hash
}

/// Returns `true` if `a` and `b` are the same length and every corresponding
/// sample differs by no more than `tolerance` (linear amplitude).
pub fn buffers_close(a: &[f32], b: &[f32], tolerance: f32) -> bool {
    a.len() == b.len()
        && a.iter()
            .zip(b.iter())
            .all(|(&x, &y)| (x - y).abs() <= tolerance)
}

/// Maximum absolute per-sample difference between `a` and `b` (linear
/// amplitude); `f32::INFINITY` if the lengths differ.
pub fn max_abs_diff(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() {
        return f32::INFINITY;
    }
    a.iter()
        .zip(b.iter())
        .map(|(&x, &y)| (x - y).abs())
        .fold(0.0, f32::max)
}

/// Asserts `a` and `b` are within `tolerance_db` of each other (via
/// [`crate::measure::null_test_db`]). Panics with a useful message
/// otherwise.
#[track_caller]
pub fn assert_null_within_db(a: &[f32], b: &[f32], tolerance_db: f64) {
    let diff_db = crate::measure::null_test_db(a, b);
    assert!(
        diff_db <= tolerance_db,
        "buffers differ by {diff_db:.3} dB, expected <= {tolerance_db:.3} dB"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn close_within_tolerance() {
        let a = [0.1_f32, 0.2, 0.3];
        let b = [0.1001_f32, 0.199, 0.301];
        assert!(buffers_close(&a, &b, 0.01));
        assert!(!buffers_close(&a, &b, 0.0001));
    }

    #[test]
    fn max_abs_diff_mismatched_lengths_is_infinite() {
        assert!(max_abs_diff(&[0.0], &[0.0, 1.0]).is_infinite());
    }
}

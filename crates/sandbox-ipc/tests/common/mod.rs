//! Shared test helpers (signals, bit comparisons).
#![allow(dead_code)]

use vox_sandbox_ipc::test_plugins::TEST_GAIN;

pub const RATE: f64 = 48_000.0;

/// Deterministic noise in [−0.5, 0.5) (no denormals: the smallest non-zero magnitude is 2⁻²⁵).
pub fn noise(n: usize, seed: u32) -> Vec<f32> {
    let mut s = seed;
    (0..n)
        .map(|_| {
            s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (s >> 8) as f32 / 16_777_216.0 - 0.5
        })
        .collect()
}

/// `amp · sin(2π f t)` at [`RATE`].
pub fn sine(n: usize, freq_hz: f64, amp: f64) -> Vec<f32> {
    (0..n)
        .map(|t| (amp * (std::f64::consts::TAU * freq_hz * t as f64 / RATE).sin()) as f32)
        .collect()
}

/// The largest sample-to-sample step.
pub fn max_step(y: &[f32]) -> f32 {
    y.windows(2)
        .map(|w| (w[1] - w[0]).abs())
        .fold(0.0, f32::max)
}

/// `x × TEST_GAIN`, exactly as the gain plugin computes it.
pub fn gained(x: &[f32]) -> Vec<f32> {
    x.iter().map(|&v| v * TEST_GAIN).collect()
}

/// Index of the first bit difference, if any (lengths must match).
pub fn first_bit_diff(got: &[f32], want: &[f32]) -> Option<usize> {
    assert_eq!(got.len(), want.len(), "length");
    got.iter()
        .zip(want)
        .position(|(a, b)| a.to_bits() != b.to_bits())
}

/// Asserts `got` is bit-identical to `want`.
pub fn assert_bits(got: &[f32], want: &[f32], what: &str) {
    if let Some(i) = first_bit_diff(got, want) {
        panic!("{what}: sample {i}: got {} want {}", got[i], want[i]);
    }
}

/// The expected pipelined output at position `t`: `x[t − latency] × TEST_GAIN`, 0 before.
pub fn expected_gain_output(x: &[f32], latency: usize) -> Vec<f32> {
    (0..x.len())
        .map(|t| {
            if t < latency {
                0.0
            } else {
                x[t - latency] * TEST_GAIN
            }
        })
        .collect()
}

//! Deterministic PRNG for tests (SplitMix64); no `rand` dependency.

/// SplitMix64. Deterministic for a given seed on every platform.
#[derive(Clone, Debug)]
pub struct TestRng(u64);

impl TestRng {
    /// Seeds the generator.
    pub fn new(seed: u64) -> Self {
        Self(seed)
    }

    /// Next 64 random bits.
    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in `[0, 1)`.
    pub fn unit_f64(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 * (1.0 / (1u64 << 53) as f64)
    }

    /// Uniform in `[-1, 1)`.
    pub fn bipolar_f32(&mut self) -> f32 {
        (self.unit_f64() * 2.0 - 1.0) as f32
    }

    /// Uniform in `0..n` (0 if `n == 0`).
    pub fn below(&mut self, n: u64) -> u64 {
        if n == 0 { 0 } else { self.next_u64() % n }
    }

    /// Uniform in `lo..=hi` (`lo` if `hi < lo`).
    pub fn range_u32(&mut self, lo: u32, hi: u32) -> u32 {
        if hi <= lo {
            return lo;
        }
        lo + self.below(u64::from(hi - lo) + 1) as u32
    }

    /// True with probability `p`.
    pub fn chance(&mut self, p: f64) -> bool {
        self.unit_f64() < p
    }
}

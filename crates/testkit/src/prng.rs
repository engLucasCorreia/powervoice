//! A tiny, deterministic, seedable PRNG used for reproducible noise
//! generation in `testkit`. Not cryptographically secure, and deliberately
//! not the `rand` crate: PCG32 XSH-RR (O'Neill 2014), ~30 lines.

const PCG_MULTIPLIER: u64 = 6364136223846793005;

/// PCG32 pseudo-random generator. The same `(seed, stream)` pair always
/// produces the same sequence, on any platform.
#[derive(Debug, Clone)]
pub struct Pcg32 {
    state: u64,
    inc: u64,
}

impl Pcg32 {
    /// Creates a generator from a `seed` and a `stream` id. Different
    /// `stream` values (e.g. one per generator) decorrelate sequences that
    /// share the same `seed`.
    pub fn new(seed: u64, stream: u64) -> Self {
        let mut rng = Pcg32 {
            state: 0,
            inc: (stream << 1) | 1,
        };
        let _ = rng.step();
        rng.state = rng.state.wrapping_add(seed);
        let _ = rng.step();
        rng
    }

    fn step(&mut self) -> u64 {
        let old_state = self.state;
        self.state = old_state
            .wrapping_mul(PCG_MULTIPLIER)
            .wrapping_add(self.inc);
        old_state
    }

    /// Next uniform `u32`.
    pub fn next_u32(&mut self) -> u32 {
        let old_state = self.step();
        let xorshifted = (((old_state >> 18) ^ old_state) >> 27) as u32;
        let rot = (old_state >> 59) as u32;
        xorshifted.rotate_right(rot)
    }

    /// Next uniform `f64` in `[0.0, 1.0)`.
    pub fn next_f64(&mut self) -> f64 {
        f64::from(self.next_u32()) / (f64::from(u32::MAX) + 1.0)
    }

    /// Next uniform `f64` in `[-1.0, 1.0)`.
    pub fn next_signed(&mut self) -> f64 {
        self.next_f64() * 2.0 - 1.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_seed_same_sequence() {
        let mut a = Pcg32::new(42, 0);
        let mut b = Pcg32::new(42, 0);
        let seq_a: Vec<u32> = (0..100).map(|_| a.next_u32()).collect();
        let seq_b: Vec<u32> = (0..100).map(|_| b.next_u32()).collect();
        assert_eq!(seq_a, seq_b);
    }

    #[test]
    fn different_seed_different_sequence() {
        let mut a = Pcg32::new(1, 0);
        let mut b = Pcg32::new(2, 0);
        let seq_a: Vec<u32> = (0..20).map(|_| a.next_u32()).collect();
        let seq_b: Vec<u32> = (0..20).map(|_| b.next_u32()).collect();
        assert_ne!(seq_a, seq_b);
    }

    #[test]
    fn signed_range_is_bounded() {
        let mut r = Pcg32::new(7, 0);
        for _ in 0..1000 {
            let v = r.next_signed();
            assert!((-1.0..1.0).contains(&v));
        }
    }

    /// Reference-vector test against O'Neill's canonical minimal-C PCG32
    /// (`pcg32_srandom_r`/`pcg32_random_r` from pcg-c-basic), seed 42, stream
    /// (`initseq`) 54. This pins our implementation to the real PCG32
    /// algorithm rather than merely to "whatever this code currently does" --
    /// verified independently by re-deriving the C reference algorithm and
    /// running it side by side with this Rust port.
    #[test]
    fn matches_canonical_pcg32_reference_vector() {
        let mut r = Pcg32::new(42, 54);
        let outputs: Vec<u32> = (0..5).map(|_| r.next_u32()).collect();
        assert_eq!(
            outputs,
            vec![0xa15c02b7, 0x7b47f409, 0xba1d3330, 0x83d2f293, 0xbfa4784b]
        );
    }
}

//! Level detectors (SPEC-016 §4.3): exact sliding peak (monotonic wedge, Lemire 2006) and a
//! drift-free rectangular sliding mean square.

use super::LEVEL_FLOOR_DBFS;

/// Exact sliding maximum of `|s|` over the last `window` samples (monotonic wedge).
///
/// Buffers are sized at construction; [`push`](Self::push) never allocates. Work per sample is
/// amortized O(1) and bounded by `window`.
#[derive(Clone, Debug)]
pub struct SlidingPeak {
    /// Ring of (sample index, magnitude), magnitudes strictly decreasing from front to back.
    ring: Box<[(u64, f64)]>,
    head: usize,
    len: usize,
    window: u64,
    /// Index of the next pushed sample.
    n: u64,
}

impl SlidingPeak {
    /// A detector over `window >= 1` samples (clamped to 1).
    pub fn new(window: usize) -> Self {
        let window = window.max(1);
        Self {
            ring: vec![(0, 0.0); window].into_boxed_slice(),
            head: 0,
            len: 0,
            window: window as u64,
            n: 0,
        }
    }

    /// Window length in samples.
    pub fn window(&self) -> usize {
        self.ring.len()
    }

    /// Empties the window (the maximum reads 0).
    pub fn reset(&mut self) {
        self.head = 0;
        self.len = 0;
        self.n = 0;
    }

    fn slot(&self, i: usize) -> usize {
        (self.head + i) % self.ring.len()
    }

    /// Pushes one magnitude (`>= 0`) and returns the maximum over the window ending at it.
    pub fn push(&mut self, magnitude: f64) -> f64 {
        let n = self.n;
        self.n += 1;
        // Expire the front (indices <= n − window).
        while self.len > 0 && self.ring[self.head].0 + self.window <= n {
            self.head = (self.head + 1) % self.ring.len();
            self.len -= 1;
        }
        // Drop smaller-or-equal values from the back: they can never be the maximum again.
        while self.len > 0 && self.ring[self.slot(self.len - 1)].1 <= magnitude {
            self.len -= 1;
        }
        let back = self.slot(self.len);
        self.ring[back] = (n, magnitude);
        self.len += 1;
        self.ring[self.head].1
    }

    /// Current maximum (0 when empty).
    pub fn max(&self) -> f64 {
        if self.len == 0 {
            0.0
        } else {
            self.ring[self.head].1
        }
    }
}

/// Rectangular sliding mean of `s²` over `window` samples, drift-free: a running `f64` sum,
/// re-summed exactly from the ring every `window` samples, and exactly 0 whenever the window
/// holds only zeros.
#[derive(Clone, Debug)]
pub struct SlidingRms {
    squares: Box<[f64]>,
    pos: usize,
    sum: f64,
    nonzero: usize,
    since_resum: usize,
}

impl SlidingRms {
    /// A detector over `window >= 1` samples (clamped to 1).
    pub fn new(window: usize) -> Self {
        Self {
            squares: vec![0.0; window.max(1)].into_boxed_slice(),
            pos: 0,
            sum: 0.0,
            nonzero: 0,
            since_resum: 0,
        }
    }

    /// Window length in samples.
    pub fn window(&self) -> usize {
        self.squares.len()
    }

    /// Empties the window (the mean square reads 0).
    pub fn reset(&mut self) {
        self.squares.fill(0.0);
        self.pos = 0;
        self.sum = 0.0;
        self.nonzero = 0;
        self.since_resum = 0;
    }

    /// Pushes one sample and returns the mean square over the window ending at it.
    pub fn push(&mut self, s: f64) -> f64 {
        let new = s * s;
        let old = std::mem::replace(&mut self.squares[self.pos], new);
        self.nonzero = self.nonzero + usize::from(new != 0.0) - usize::from(old != 0.0);
        self.sum += new - old;
        self.pos = (self.pos + 1) % self.squares.len();
        self.since_resum += 1;
        if self.since_resum == self.squares.len() {
            self.since_resum = 0;
            self.sum = self.squares.iter().sum();
        }
        if self.nonzero == 0 || self.sum < 0.0 {
            self.sum = 0.0;
        }
        self.mean_square()
    }

    /// Current mean square.
    pub fn mean_square(&self) -> f64 {
        self.sum / self.squares.len() as f64
    }
}

/// Exact look-ahead delay line on the audio path (SPEC-016 §4.9): the sample returned is the
/// one pushed `len` samples earlier, bit-identical. `len = 0` is a pass-through.
///
/// The buffer is sized at construction; [`push`](Self::push) never allocates.
#[derive(Clone, Debug)]
pub struct DelayLine {
    buf: Box<[f32]>,
    pos: usize,
}

impl DelayLine {
    /// A cleared line of `len` samples (`0` = pass-through).
    pub fn new(len: usize) -> Self {
        Self {
            buf: vec![0.0; len].into_boxed_slice(),
            pos: 0,
        }
    }

    /// Delay in samples.
    pub fn len(&self) -> usize {
        self.buf.len()
    }

    /// True for a pass-through line (delay 0).
    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }

    /// Zeroes the line (reset / activate).
    pub fn clear(&mut self) {
        self.buf.fill(0.0);
        self.pos = 0;
    }

    /// Pushes one sample and returns the one from `len` samples ago (0 while the line fills).
    pub fn push(&mut self, x: f32) -> f32 {
        if self.buf.is_empty() {
            return x;
        }
        let y = std::mem::replace(&mut self.buf[self.pos], x);
        self.pos += 1;
        if self.pos == self.buf.len() {
            self.pos = 0;
        }
        y
    }
}

/// Mean square → RMS level in dB (`10·log10`), clamped to [`LEVEL_FLOOR_DBFS`].
pub fn mean_square_to_db(ms: f64) -> f64 {
    if ms > 0.0 {
        (10.0 * ms.log10()).max(LEVEL_FLOOR_DBFS)
    } else {
        LEVEL_FLOOR_DBFS
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Tiny deterministic generator (xorshift64*), so dsp needs no test dependency.
    pub(crate) fn noise(seed: u64, n: usize) -> Vec<f64> {
        let mut s = seed.max(1);
        (0..n)
            .map(|_| {
                s ^= s >> 12;
                s ^= s << 25;
                s ^= s >> 27;
                let r = s.wrapping_mul(0x2545_F491_4F6C_DD1D);
                (r >> 11) as f64 / (1u64 << 53) as f64 * 2.0 - 1.0
            })
            .collect()
    }

    #[test]
    fn sliding_peak_matches_brute_force() {
        for window in [1usize, 2, 7, 240] {
            let x = noise(window as u64 + 3, 5000);
            let mut d = SlidingPeak::new(window);
            for (i, v) in x.iter().enumerate() {
                // Occasional runs of equal and decreasing values stress the wedge.
                let m = if i % 97 < 10 { 0.5 } else { v.abs() };
                let got = d.push(m);
                let lo = (i + 1).saturating_sub(window);
                let want = (lo..=i)
                    .map(|j| if j % 97 < 10 { 0.5 } else { x[j].abs() })
                    .fold(0.0, f64::max);
                assert_eq!(got.to_bits(), want.to_bits(), "window {window} sample {i}");
            }
        }
    }

    #[test]
    fn sliding_peak_reset_empties() {
        let mut d = SlidingPeak::new(4);
        d.push(0.9);
        d.reset();
        assert_eq!(d.max().to_bits(), 0f64.to_bits());
        assert_eq!(d.push(0.1).to_bits(), 0.1f64.to_bits());
    }

    #[test]
    fn delay_line_is_exact_and_clears() {
        let mut d = DelayLine::new(0);
        assert!(d.is_empty());
        assert_eq!(
            d.push(0.25).to_bits(),
            0.25f32.to_bits(),
            "len 0 passes through"
        );
        for len in [1usize, 3, 240] {
            let mut d = DelayLine::new(len);
            assert_eq!(d.len(), len);
            let x: Vec<f32> = (0..1000).map(|i| (i as f32) * 1.000_001).collect();
            for (i, &v) in x.iter().enumerate() {
                let got = d.push(v);
                let want = if i >= len { x[i - len] } else { 0.0 };
                assert_eq!(got.to_bits(), want.to_bits(), "len {len} sample {i}");
            }
            d.clear();
            assert_eq!(d.push(1.0).to_bits(), 0f32.to_bits());
        }
    }

    #[test]
    fn sliding_rms_matches_brute_force_and_is_exact_on_silence() {
        let window = 960;
        let mut x = noise(11, 20_000);
        for v in &mut x[12_000..14_000] {
            *v = 0.0;
        }
        let mut d = SlidingRms::new(window);
        for (i, &v) in x.iter().enumerate() {
            let got = d.push(v);
            let lo = (i + 1).saturating_sub(window);
            let want = x[lo..=i].iter().map(|v| v * v).sum::<f64>() / window as f64;
            assert!(
                (got - want).abs() <= 1e-12 * want.max(1e-300),
                "sample {i}: {got} vs {want}"
            );
            if (12_000 + window - 1..14_000).contains(&i) {
                assert_eq!(
                    got.to_bits(),
                    0f64.to_bits(),
                    "silent window must read exactly 0"
                );
            }
        }
        assert!((mean_square_to_db(0.5) + 3.010_299_956_639_812).abs() < 1e-12);
        assert!((mean_square_to_db(0.0) - LEVEL_FLOOR_DBFS).abs() < f64::EPSILON);
    }
}

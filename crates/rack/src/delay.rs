//! Ring-buffer delay line with read taps (latency-matched dry paths: per-slot bypass and
//! whole-rack A/B). Allocated off the audio thread; every other method is RT-safe.

/// Delay line holding the last `capacity` input samples. A block of `n` samples is written with
/// [`write`](Self::write); taps `d` with `d + n <= capacity` can then be read for that block.
#[derive(Debug, Default)]
pub(crate) struct DelayLine {
    buf: Vec<f32>,
    /// Index the next sample is written to.
    write: usize,
    max_tap: usize,
}

impl DelayLine {
    /// \[control thread\] A line serving taps up to `max_tap` for blocks up to `max_block`.
    /// `max_tap == 0` allocates nothing (tap 0 is the input itself).
    pub(crate) fn new(max_tap: usize, max_block: usize) -> Self {
        if max_tap == 0 {
            return Self::default();
        }
        Self {
            buf: vec![0.0; max_tap + max_block],
            write: 0,
            max_tap,
        }
    }

    /// Largest supported tap.
    pub(crate) fn max_tap(&self) -> usize {
        self.max_tap
    }

    /// Appends `input` (at most `capacity - max_tap` samples); returns the index of `input[0]`,
    /// to be passed to the reads of this block.
    pub(crate) fn write(&mut self, input: &[f32]) -> usize {
        let base = self.write;
        let cap = self.buf.len();
        if cap == 0 {
            return 0;
        }
        let first = input.len().min(cap - base);
        self.buf[base..base + first].copy_from_slice(&input[..first]);
        let rest = input.len() - first;
        self.buf[..rest].copy_from_slice(&input[first..]);
        self.write = (base + input.len()) % cap;
        base
    }

    /// Sample `i` of the block written at `base`, delayed by `tap` (`1 <= tap <= max_tap`).
    #[inline]
    pub(crate) fn get(&self, base: usize, i: usize, tap: usize) -> f32 {
        let cap = self.buf.len();
        self.buf[(base + i + cap - tap) % cap]
    }

    /// Copies the block written at `base`, delayed by `tap` (`1 <= tap <= max_tap`), into `out`.
    pub(crate) fn read_into(&self, base: usize, tap: usize, out: &mut [f32]) {
        let cap = self.buf.len();
        let start = (base + cap - tap) % cap;
        let first = out.len().min(cap - start);
        out[..first].copy_from_slice(&self.buf[start..start + first]);
        let rest = out.len() - first;
        out[first..].copy_from_slice(&self.buf[..rest]);
    }

    /// Zeroes the history (seek / transport start).
    pub(crate) fn clear(&mut self) {
        self.buf.fill(0.0);
    }

    /// Takes over the most recent history of `other` (as much as both hold), so taps read the
    /// same samples as they would have from `other`. Bounded copy; RT-safe.
    pub(crate) fn copy_history_from(&mut self, other: &DelayLine) {
        let (cap, ocap) = (self.buf.len(), other.buf.len());
        if cap == 0 || ocap == 0 {
            return;
        }
        let n = cap.min(ocap);
        for j in 1..=n {
            self.buf[(self.write + cap - j) % cap] = other.buf[(other.write + ocap - j) % ocap];
        }
    }
}

#[cfg(test)]
#[allow(clippy::float_cmp, clippy::needless_range_loop)] // exact samples, indexed by time
mod tests {
    use super::*;

    #[test]
    fn taps_read_the_past() {
        let mut d = DelayLine::new(5, 4);
        let mut out = [0.0; 4];
        let mut all = Vec::new();
        for b in 0..6 {
            let block: Vec<f32> = (0..4).map(|i| (b * 4 + i + 1) as f32).collect();
            let base = d.write(&block);
            all.extend_from_slice(&block);
            d.read_into(base, 5, &mut out);
            for i in 0..4 {
                let n = b * 4 + i;
                let want = if n >= 5 { all[n - 5] } else { 0.0 };
                assert_eq!(out[i], want);
                assert_eq!(d.get(base, i, 5), want);
                let want3 = if n >= 3 { all[n - 3] } else { 0.0 };
                assert_eq!(d.get(base, i, 3), want3);
            }
        }
    }

    #[test]
    fn history_copy() {
        let mut a = DelayLine::new(3, 2);
        for b in 0..5 {
            a.write(&[b as f32 * 2.0, b as f32 * 2.0 + 1.0]);
        }
        let mut b = DelayLine::new(8, 2);
        b.copy_history_from(&a);
        let base = b.write(&[100.0, 101.0]);
        assert_eq!(b.get(base, 0, 1), 9.0);
        assert_eq!(b.get(base, 0, 3), 7.0);
        assert_eq!(b.get(base, 1, 1), 100.0);
    }
}

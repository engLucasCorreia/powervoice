//! Look-ahead gain computer (SPEC-017 §4.3): min-hold over L+1 requirements, linear-in-dB
//! release, two cascaded moving averages of length L/2 + 1.
//!
//! Every value averaged into `G[n]` is at most `min(r[n − L ..= n])`, so the gain never exceeds
//! any requirement inside its window ("under the ceiling by construction"). A window of all 1.0
//! yields exactly 1.0 (non-unity counters), which makes below-ceiling material bit-exact.
//!
//! Buffers are allocated in [`LookaheadGain::new`]; [`push`](LookaheadGain::push) and
//! [`reset`](LookaheadGain::reset) are RT-safe.

/// `x == 1.0` exactly (unity is the exact rest value).
#[allow(clippy::float_cmp)]
fn is_unity(x: f64) -> bool {
    x == 1.0
}

/// Moving average whose output is exactly 1.0 while its window holds only 1.0.
#[derive(Clone, Debug)]
struct MovingAverage {
    buf: Box<[f64]>,
    pos: usize,
    sum: f64,
    non_unity: usize,
}

impl MovingAverage {
    fn new(len: usize) -> Self {
        let len = len.max(1);
        Self {
            buf: vec![1.0; len].into_boxed_slice(),
            pos: 0,
            sum: len as f64,
            non_unity: 0,
        }
    }

    fn reset(&mut self) {
        self.buf.fill(1.0);
        self.pos = 0;
        self.sum = self.buf.len() as f64;
        self.non_unity = 0;
    }

    #[inline]
    fn push(&mut self, x: f64) -> f64 {
        let n = self.buf.len();
        let old = std::mem::replace(&mut self.buf[self.pos], x);
        self.pos += 1;
        if self.pos == n {
            self.pos = 0;
        }
        if !is_unity(old) {
            self.non_unity -= 1;
        }
        if !is_unity(x) {
            self.non_unity += 1;
        }
        if self.non_unity == 0 {
            self.sum = n as f64;
            return 1.0;
        }
        if self.pos == 0 {
            // Re-sum exactly once per window length so the running sum cannot drift.
            self.sum = self.buf.iter().sum();
        } else {
            self.sum += x - old;
        }
        (self.sum / n as f64).min(1.0)
    }
}

/// Gain computer: requirement `r ∈ (0, 1]` in, applied gain `G ∈ (0, 1]` out, one per sample.
#[derive(Clone, Debug)]
pub struct LookaheadGain {
    lookahead: usize,
    /// Monotonic min-deque (ring): sample indices and requirements, values increasing.
    dq_index: Box<[u64]>,
    dq_value: Box<[f64]>,
    dq_head: usize,
    dq_len: usize,
    t: u64,
    release_coeff: f64,
    env: f64,
    ma1: MovingAverage,
    ma2: MovingAverage,
}

impl LookaheadGain {
    /// A gain computer for look-ahead `L = lookahead` samples (even) and release coefficient
    /// `k_r > 1` (per-sample gain recovery factor). Allocates.
    pub fn new(lookahead: usize, release_coeff: f64) -> Self {
        let cap = lookahead + 1;
        let avg = lookahead / 2 + 1;
        Self {
            lookahead,
            dq_index: vec![0; cap].into_boxed_slice(),
            dq_value: vec![1.0; cap].into_boxed_slice(),
            dq_head: 0,
            dq_len: 0,
            t: 0,
            release_coeff,
            env: 1.0,
            ma1: MovingAverage::new(avg),
            ma2: MovingAverage::new(avg),
        }
    }

    /// Look-ahead L in samples.
    pub fn lookahead(&self) -> usize {
        self.lookahead
    }

    /// Sets the per-sample release factor `k_r` (takes effect with the next sample). RT-safe.
    pub fn set_release_coeff(&mut self, k: f64) {
        self.release_coeff = k;
    }

    /// Back to rest: G = 1, hold, release and averages cleared. RT-safe.
    pub fn reset(&mut self) {
        self.dq_head = 0;
        self.dq_len = 0;
        self.env = 1.0;
        self.ma1.reset();
        self.ma2.reset();
    }

    /// Pushes the requirement of the newest interval and returns the gain for the sample
    /// `L` samples older. RT-safe, O(1) amortized.
    #[inline]
    pub fn push(&mut self, r: f64) -> f64 {
        let cap = self.dq_index.len();
        let t = self.t;
        self.t = t + 1;
        // Expire entries older than the window [t − L, t].
        while self.dq_len > 0 && self.dq_index[self.dq_head] + cap as u64 <= t {
            self.dq_head = if self.dq_head + 1 == cap {
                0
            } else {
                self.dq_head + 1
            };
            self.dq_len -= 1;
        }
        // 1.0 never lowers the minimum: an empty deque means h = 1.
        if r < 1.0 {
            while self.dq_len > 0 {
                let back = (self.dq_head + self.dq_len - 1) % cap;
                if self.dq_value[back] >= r {
                    self.dq_len -= 1;
                } else {
                    break;
                }
            }
            let slot = (self.dq_head + self.dq_len) % cap;
            self.dq_index[slot] = t;
            self.dq_value[slot] = r;
            self.dq_len += 1;
        }
        let hold = if self.dq_len > 0 {
            self.dq_value[self.dq_head]
        } else {
            1.0
        };
        let recovered = (self.env * self.release_coeff).min(1.0);
        self.env = hold.min(recovered);
        let g = self.ma1.push(self.env);
        self.ma2.push(g)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::float_cmp)] // exact unity is the property under test
    use super::*;
    use crate::true_peak::release_coeff;

    fn lcg(state: &mut u64) -> f64 {
        *state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (*state >> 11) as f64 / (1u64 << 53) as f64
    }

    #[test]
    fn unity_in_gives_exact_unity_out() {
        let mut g = LookaheadGain::new(240, release_coeff(100.0, 48_000.0));
        for _ in 0..10_000 {
            assert_eq!(g.push(1.0), 1.0);
        }
    }

    /// Output n belongs to the requirement pushed L samples earlier: every value averaged into it
    /// held that requirement, so G[n] ≤ r[n − L] (by construction).
    #[test]
    fn gain_never_exceeds_a_requirement_in_its_window() {
        let l = 48;
        let mut g = LookaheadGain::new(l, release_coeff(10.0, 48_000.0));
        let mut rng = 7u64;
        let mut r = Vec::new();
        let mut out = Vec::new();
        for n in 0..20_000usize {
            let v = if (n / 700).is_multiple_of(2) && lcg(&mut rng) < 0.05 {
                0.05 + 0.9 * lcg(&mut rng)
            } else {
                1.0
            };
            r.push(v);
            out.push(g.push(v));
        }
        // Output n is aligned with requirement n − L (look-ahead): it must honour r[n − L].
        for n in l..out.len() {
            assert!(out[n] <= r[n - l] * (1.0 + 1e-12), "n {n}");
            assert!(out[n] > 0.0 && out[n] <= 1.0);
        }
    }

    #[test]
    fn attack_is_monotonic_and_spans_l_plus_one_samples() {
        let l = 240;
        let mut g = LookaheadGain::new(l, release_coeff(100.0, 48_000.0));
        let mut out = Vec::new();
        for n in 0..2_000 {
            out.push(g.push(if n >= 1_000 { 0.5 } else { 1.0 }));
        }
        assert_eq!(out[999], 1.0);
        assert!(out[1_000] < 1.0);
        for n in 1_000..1_000 + l {
            assert!(out[n + 1] <= out[n]);
        }
        assert!((out[1_000 + l] - 0.5).abs() < 1e-12, "{}", out[1_000 + l]);
        assert!(out[1_000 + l - 1] > 0.5);
    }

    fn release_ms(release: f64, rate: f64) -> (f64, usize, usize) {
        let l = 240;
        let mut g = LookaheadGain::new(l, release_coeff(release, rate));
        let low = 10f64.powf(-13.0 / 20.0);
        let n_total = (rate * (release / 1000.0 * 1.5 + 0.5)) as usize;
        let drop_at = (0.3 * rate) as usize;
        let out: Vec<f64> = (0..n_total)
            .map(|n| g.push(if n < drop_at { low } else { 1.0 }))
            .collect();
        let db: Vec<f64> = out.iter().map(|x| 20.0 * x.log10()).collect();
        let crossing = |level: f64| {
            let i = (drop_at..db.len()).find(|&i| db[i] >= level).unwrap();
            (i - 1) as f64 + (level - db[i - 1]) / (db[i] - db[i - 1])
        };
        let t = (crossing(-0.5) - crossing(-12.5)) / rate * 1000.0;
        let last_below = (drop_at..out.len()).rev().find(|&i| out[i] < 1.0).unwrap();
        let at_half = (drop_at..db.len()).find(|&i| db[i] >= -0.5).unwrap();
        (t, last_below, at_half)
    }

    #[test]
    fn release_is_twelve_db_per_release_time() {
        let rate = 48_000.0;
        for release in [100.0, 1_000.0] {
            let (t, last_below, at_half) = release_ms(release, rate);
            assert!(
                (t / release - 1.0).abs() <= 0.02,
                "release {release}: {t} ms"
            );
            // Exactly 1.0 no later than release/12 + look-ahead after the −0.5 dB crossing.
            let limit = ((release / 12.0 + 5.0) / 1000.0 * rate).ceil() as usize;
            assert!(last_below < at_half + limit, "release {release}");
        }
        let (t, _, _) = release_ms(10.0, rate);
        assert!((10.0..=12.0).contains(&t), "release 10: {t} ms");
    }
}

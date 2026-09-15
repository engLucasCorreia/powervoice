//! T-110: a preallocated, lock-free callback-time histogram (ADR-002 §2 follow-up: "a
//! callback-time histogram using the permitted `Instant` read"). Every bucket is a fixed-size
//! atomic array — [`Histogram::record`] never allocates, locks or blocks, so it is safe to call
//! from the code path an RT guard wraps around a real output callback
//! (`FakeBackend::set_rt_guard`, `crates/engine/benches/callback_histogram.rs`).
//!
//! Buckets are power-of-two width in nanoseconds: bucket `i` covers `[2^(i-1), 2^i - 1]` ns
//! (bucket 0 is exactly 0 ns). [`Histogram::percentile`] reports its bucket's **upper** bound,
//! so it never *understates* a callback's cost — the usual convention for a bucketed histogram
//! used as a budget check — at the cost of reading up to ~2x high for the top (least-populated)
//! bucket. [`Histogram::max`] instead tracks the exact maximum (a single `AtomicU64::fetch_max`),
//! so it can occasionally read *below* a bucket-quantized [`Histogram::percentile`] that lands in
//! the same top bucket; it is never below the true maximum itself.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// One bucket per possible `u64::leading_zeros()` result (0..=64).
const BUCKETS: usize = 65;

/// A lock-free, allocation-free histogram of durations, in nanoseconds.
pub struct Histogram {
    buckets: [AtomicU64; BUCKETS],
    count: AtomicU64,
    max_ns: AtomicU64,
}

impl Histogram {
    /// An empty histogram.
    pub const fn new() -> Self {
        Self {
            buckets: [const { AtomicU64::new(0) }; BUCKETS],
            count: AtomicU64::new(0),
            max_ns: AtomicU64::new(0),
        }
    }

    /// Records one duration. Lock-free, allocation-free: three atomic RMW ops.
    pub fn record(&self, elapsed: Duration) {
        let ns = elapsed.as_nanos().min(u128::from(u64::MAX)) as u64;
        self.buckets[bucket_index(ns)].fetch_add(1, Ordering::Relaxed);
        self.max_ns.fetch_max(ns, Ordering::Relaxed);
        self.count.fetch_add(1, Ordering::Relaxed);
    }

    /// Samples recorded so far.
    pub fn count(&self) -> u64 {
        self.count.load(Ordering::Relaxed)
    }

    /// The exact maximum recorded duration, or `None` if empty.
    pub fn max(&self) -> Option<Duration> {
        if self.count() == 0 {
            None
        } else {
            Some(Duration::from_nanos(self.max_ns.load(Ordering::Relaxed)))
        }
    }

    /// The `p`-th percentile (`0.0..=1.0`, e.g. `0.99` for p99) as its bucket's upper bound: the
    /// smallest bucket boundary such that at least `p` of recorded samples are at or below it.
    /// `None` if empty or `p` is out of range.
    pub fn percentile(&self, p: f64) -> Option<Duration> {
        let total = self.count();
        if total == 0 || !(0.0..=1.0).contains(&p) {
            return None;
        }
        let target = ((p * total as f64).ceil() as u64).clamp(1, total);
        let mut cumulative = 0u64;
        for (i, bucket) in self.buckets.iter().enumerate() {
            cumulative += bucket.load(Ordering::Relaxed);
            if cumulative >= target {
                return Some(Duration::from_nanos(bucket_upper_ns(i)));
            }
        }
        // Unreachable in practice (cumulative reaches `total >= target` by the last bucket), but
        // never panics on a valid, non-empty histogram (RT-adjacent code path).
        self.max()
    }
}

impl Default for Histogram {
    fn default() -> Self {
        Self::new()
    }
}

/// `i` such that `ns` falls in bucket `i`'s `[2^(i-1), 2^i - 1]` range (bucket 0: `ns == 0`).
fn bucket_index(ns: u64) -> usize {
    (u64::BITS - ns.leading_zeros()) as usize
}

/// The upper (inclusive) bound of bucket `i`, in nanoseconds.
fn bucket_upper_ns(i: usize) -> u64 {
    if i == 0 { 0 } else { (1u64 << i) - 1 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_reports_nothing() {
        let h = Histogram::new();
        assert_eq!(h.count(), 0);
        assert_eq!(h.max(), None);
        assert_eq!(h.percentile(0.5), None);
    }

    #[test]
    fn single_sample_is_its_own_every_percentile_and_exact_max() {
        let h = Histogram::new();
        h.record(Duration::from_micros(100));
        assert_eq!(h.count(), 1);
        // `max` is exact (not bucket-quantized).
        assert_eq!(h.max(), Some(Duration::from_micros(100)));
        // `percentile` is bucket-quantized: 100_000 ns falls in [65536, 131071], upper bound
        // 131071 ns — every percentile of one sample names the same (that sample's) bucket.
        let quantized = Some(Duration::from_nanos(131_071));
        assert_eq!(h.percentile(0.01), quantized);
        assert_eq!(h.percentile(0.50), quantized);
        assert_eq!(h.percentile(0.99), quantized);
    }

    #[test]
    fn zero_duration_is_bucket_zero() {
        let h = Histogram::new();
        h.record(Duration::ZERO);
        assert_eq!(h.max(), Some(Duration::ZERO));
        assert_eq!(h.percentile(0.5), Some(Duration::ZERO));
    }

    #[test]
    fn percentiles_are_monotonic_and_track_the_exact_max() {
        let h = Histogram::new();
        // A spread of durations, including one clear outlier.
        for ns in [100u64, 200, 300, 400, 500, 600, 700, 800, 900, 50_000] {
            h.record(Duration::from_nanos(ns));
        }
        assert_eq!(h.count(), 10);
        let p50 = h.percentile(0.50).unwrap();
        let p95 = h.percentile(0.95).unwrap();
        let p99 = h.percentile(0.99).unwrap();
        let max = h.max().unwrap();
        assert!(p50 <= p95, "p50 {p50:?} > p95 {p95:?}");
        assert!(p95 <= p99, "p95 {p95:?} > p99 {p99:?}");
        // The outlier at 50_000 ns is the true (unquantized) max; 9/10 samples are far below it.
        assert_eq!(max, Duration::from_nanos(50_000));
        // p99 (10/10 samples, the outlier's own bucket) is bucket-quantized so it can read a
        // little *above* the exact max — never below it, and bounded by the bucket's width
        // (less than 2x its lower edge, so under 2x the true max here).
        assert!(
            p99 >= max,
            "p99 {p99:?} < exact max {max:?} (must never understate)"
        );
        assert!(p99 < max * 2, "p99 {p99:?} too far above max {max:?}");
        assert!(p50 < Duration::from_nanos(2_000), "p50 {p50:?} too high");
    }

    #[test]
    fn reported_values_never_understate_the_true_duration() {
        let h = Histogram::new();
        let samples = [1u64, 3, 7, 15, 16, 17, 1023, 1024, 1_000_000, u64::MAX];
        for &ns in &samples {
            h.record(Duration::from_nanos(ns));
        }
        for &ns in &samples {
            // Every recorded value is at or under the reported max (bucket upper bound).
            assert!(h.max().unwrap().as_nanos() >= u128::from(ns));
        }
    }

    #[test]
    fn bucket_index_covers_power_of_two_boundaries() {
        assert_eq!(bucket_index(0), 0);
        assert_eq!(bucket_index(1), 1);
        assert_eq!(bucket_index(2), 2);
        assert_eq!(bucket_index(3), 2);
        assert_eq!(bucket_index(4), 3);
        assert_eq!(bucket_index(7), 3);
        assert_eq!(bucket_index(8), 4);
        assert_eq!(bucket_upper_ns(0), 0);
        assert_eq!(bucket_upper_ns(1), 1);
        assert_eq!(bucket_upper_ns(2), 3);
        assert_eq!(bucket_upper_ns(3), 7);
        assert_eq!(bucket_upper_ns(4), 15);
    }

    #[test]
    fn out_of_range_percentile_is_none() {
        let h = Histogram::new();
        h.record(Duration::from_micros(10));
        assert_eq!(h.percentile(-0.1), None);
        assert_eq!(h.percentile(1.1), None);
    }
}

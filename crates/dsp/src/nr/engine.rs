//! Streaming STFT noise reducer (SPEC-014 §4.4–§4.7).
//!
//! Framing (normative, block-size independent): a sample counter `c` runs from 0 at
//! construction/[`reset`](SpectralNr::reset); input before `c = 0` is zero. Frame `j ≥ 1` covers
//! input `[jH − N, jH)` and is processed after input `jH − 1`, before output `jH`; output `c`
//! carries overlap-add position `c − N`, so the latency is exactly N. The parameters in force at
//! every hop boundary are kept in a ring of `N/H + 1 = 5` snapshots and frame `j` uses the one of
//! its first input sample `jH − N`: a change at input sample k can only affect output ≥ k + N.

use std::sync::Arc;

use realfft::num_complex::Complex;
use realfft::{ComplexToReal, RealFftPlanner, RealToComplex};

/// Decision-directed smoothing β (Ephraim–Malah).
pub const BETA: f32 = 0.98;
/// Lower bound of the a-priori SNR ξ (−60 dB), a numerical guard.
pub const XI_MIN: f32 = 1e-6;
/// Bins whose effective noise power `λ_e` is below this pass unchanged (g = 1).
pub const LAMBDA_FLOOR: f32 = 1e-24;
/// Clamp of γ and ξ.
const SNR_MAX: f32 = 1e12;
/// Per-bin states below this magnitude are flushed to 0 (denormal safety).
const FLUSH: f32 = 1e-30;
/// Parameter snapshots kept: `N/H + 1`.
const RING: usize = 5;
/// dB → natural-log factor for amplitudes: `ln(10)/20`.
const DB_TO_LN: f32 = std::f32::consts::LN_10 / 20.0;

/// Plain parameter values a frame is processed with (SPEC-014 §3.1).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NrParams {
    /// Reduce by, dB (gain floor `G_min = 10^(−r/20)`).
    pub reduction_db: f64,
    /// Noise reduction, % of the reduction (in dB) applied.
    pub amount_pct: f64,
    /// Output `(1 − g)·Y` instead of `g·Y`.
    pub noise_only: bool,
    /// The print is raised by this much before the gain rule.
    pub sensitivity_db: f64,
    /// Width of the frequency smoothing of the gains (0 = off).
    pub smoothing_hz: f64,
    /// Rise time constant of a bin's gain.
    pub attack_ms: f64,
    /// Fall time constant of a bin's gain.
    pub release_ms: f64,
}

impl Default for NrParams {
    /// SPEC-014 §3.1 defaults.
    fn default() -> Self {
        Self {
            reduction_db: 12.0,
            amount_pct: 100.0,
            noise_only: false,
            sensitivity_db: 3.0,
            smoothing_hz: 100.0,
            attack_ms: 5.0,
            release_ms: 100.0,
        }
    }
}

/// FFT plans, buffers and per-bin state; only present with a print.
struct Stft {
    fwd: Arc<dyn RealToComplex<f32>>,
    inv: Arc<dyn ComplexToReal<f32>>,
    scratch: Vec<Complex<f32>>,
    frame: Vec<f32>,
    spec: Vec<Complex<f32>>,
    /// √(periodic Hann).
    win_a: Box<[f32]>,
    /// √(periodic Hann) × ½ / N: synthesis window with the inverse-FFT scale and the
    /// overlap-add gain (Σ Hann(n − jH) = 2) folded in.
    win_s: Box<[f32]>,
    /// Noise power per bin λ(k).
    lambda: Box<[f32]>,
    /// |Y|² of the previous frame.
    p_prev: Box<[f32]>,
    /// Full-strength gain ĝ = 10^(Ĝ/20) of the previous frame (DD history, before the
    /// Noise reduction % scaling).
    g_prev: Box<[f32]>,
    /// Time-smoothed gain Ĝ, dB.
    g_hat: Box<[f32]>,
    /// Per-frame gain G_dB (scratch).
    g_db: Box<[f32]>,
    /// Prefix sums for the frequency smoothing (scratch).
    prefix: Box<[f64]>,
    /// Next frame is the first after construction/reset (DD without history, smoothers snap).
    first: bool,
}

impl Stft {
    fn new(n: usize, lambda: Vec<f32>) -> Self {
        let mut planner = RealFftPlanner::<f32>::new();
        let fwd = planner.plan_fft_forward(n);
        let inv = planner.plan_fft_inverse(n);
        let scratch_len = fwd.get_scratch_len().max(inv.get_scratch_len());
        let bins = n / 2 + 1;
        let root_hann: Vec<f64> = (0..n)
            .map(|i| (std::f64::consts::PI * i as f64 / n as f64).sin())
            .collect();
        Self {
            scratch: vec![Complex::default(); scratch_len],
            frame: fwd.make_input_vec(),
            spec: fwd.make_output_vec(),
            fwd,
            inv,
            win_a: root_hann.iter().map(|&w| w as f32).collect(),
            win_s: root_hann
                .iter()
                .map(|&w| (w * 0.5 / n as f64) as f32)
                .collect(),
            lambda: lambda.into_boxed_slice(),
            p_prev: vec![0.0; bins].into_boxed_slice(),
            g_prev: vec![0.0; bins].into_boxed_slice(),
            g_hat: vec![0.0; bins].into_boxed_slice(),
            g_db: vec![0.0; bins].into_boxed_slice(),
            prefix: vec![0.0; bins + 1].into_boxed_slice(),
            first: true,
        }
    }

    fn clear(&mut self) {
        self.p_prev.fill(0.0);
        self.g_prev.fill(0.0);
        self.g_hat.fill(0.0);
        self.first = true;
    }

    /// Analyses the frame ending at the current count (input history `hist`, oldest sample at
    /// `base`), applies the gains and overlap-adds into `ola` (output slots from `base`).
    fn frame(
        &mut self,
        hist: &[f32],
        ola: &mut [f32],
        base: usize,
        p: &NrParams,
        sample_rate: f64,
        hop: usize,
    ) {
        let split = hist.len() - base;
        let (first, second) = self.frame.split_at_mut(split);
        for ((f, &h), &w) in first
            .iter_mut()
            .zip(&hist[base..])
            .zip(&self.win_a[..split])
        {
            *f = h * w;
        }
        for ((f, &h), &w) in second
            .iter_mut()
            .zip(&hist[..base])
            .zip(&self.win_a[split..])
        {
            *f = h * w;
        }
        // Buffer lengths come from the plans, so these cannot fail.
        let _ = self
            .fwd
            .process_with_scratch(&mut self.frame, &mut self.spec, &mut self.scratch);
        self.apply_gains(p, sample_rate, hist.len(), hop);
        if let Some(c) = self.spec.first_mut() {
            c.im = 0.0;
        }
        if let Some(c) = self.spec.last_mut() {
            c.im = 0.0;
        }
        let _ = self
            .inv
            .process_with_scratch(&mut self.spec, &mut self.frame, &mut self.scratch);
        let (first, second) = self.frame.split_at(split);
        for ((o, &f), &w) in ola[base..].iter_mut().zip(first).zip(&self.win_s[..split]) {
            *o += f * w;
        }
        for ((o, &f), &w) in ola[..base].iter_mut().zip(second).zip(&self.win_s[split..]) {
            *o += f * w;
        }
    }

    /// SPEC-014 §4.5 steps 2–9 on `self.spec`.
    #[allow(clippy::needless_range_loop)] // parallel per-bin arrays
    fn apply_gains(&mut self, p: &NrParams, sample_rate: f64, n: usize, hop: usize) {
        let bins = self.spec.len();
        let sens = 10f64.powf(p.sensitivity_db / 10.0) as f32;
        let floor_db = -(p.reduction_db.max(0.0) as f32);
        let amount = (p.amount_pct / 100.0).clamp(0.0, 1.0) as f32;
        let half_width = (p.smoothing_hz.max(0.0) / (2.0 * sample_rate / n as f64)).round();
        let h = if half_width.is_finite() {
            (half_width as usize).min(bins)
        } else {
            0
        };
        let coef = |ms: f64| (-(hop as f64) / (sample_rate * ms.max(1e-3) / 1000.0)).exp() as f32;
        let (a_att, a_rel) = (coef(p.attack_ms), coef(p.release_ms));
        let first = self.first;

        // Steps 2–5: a-posteriori SNR, decision-directed a-priori SNR, floored Wiener gain (dB).
        for k in 0..bins {
            let y = self.spec[k];
            let power = y.re * y.re + y.im * y.im;
            let lam_e = self.lambda[k] * sens;
            self.g_db[k] = if lam_e >= LAMBDA_FLOOR {
                let gamma = (power / lam_e).min(SNR_MAX);
                let post = (gamma - 1.0).max(0.0);
                let xi = if first {
                    post
                } else {
                    let g = self.g_prev[k];
                    BETA * g * g * self.p_prev[k] / lam_e + (1.0 - BETA) * post
                };
                // Written out (not `clamp`) so a NaN maps to ξ_min.
                let xi = if xi >= XI_MIN {
                    xi.min(SNR_MAX)
                } else {
                    XI_MIN
                };
                (20.0 * (xi / (1.0 + xi)).log10()).max(floor_db)
            } else {
                0.0
            };
            self.p_prev[k] = if power >= FLUSH { power } else { 0.0 };
        }

        // Step 6: frequency smoothing, mean over k ± h (truncated at the edges).
        if h > 0 {
            let mut acc = 0.0f64;
            self.prefix[0] = 0.0;
            for k in 0..bins {
                acc += f64::from(self.g_db[k]);
                self.prefix[k + 1] = acc;
            }
            for k in 0..bins {
                let lo = k.saturating_sub(h);
                let hi = (k + h).min(bins - 1);
                let mean = (self.prefix[hi + 1] - self.prefix[lo]) / (hi - lo + 1) as f64;
                self.g_db[k] = mean as f32;
            }
        }

        // Steps 7–9: attack/release smoothing in dB (snap on the first frame), amount, apply.
        for k in 0..bins {
            let target = self.g_db[k];
            let g_hat = if first {
                target
            } else {
                let prev = self.g_hat[k];
                let a = if target > prev { a_att } else { a_rel };
                prev + (1.0 - a) * (target - prev)
            };
            let g_hat = if g_hat.abs() >= FLUSH { g_hat } else { 0.0 };
            self.g_hat[k] = g_hat;
            let (g_full, g) = if self.lambda[k] * sens >= LAMBDA_FLOOR {
                ((g_hat * DB_TO_LN).exp(), (amount * g_hat * DB_TO_LN).exp())
            } else {
                (1.0, 1.0)
            };
            // DD history tracks the full-strength gain (identical to g at 100 %), so the
            // amount is a pure dB-domain dry/wet of the estimator (SPEC-014 §3.1, AC-8).
            self.g_prev[k] = g_full;
            self.spec[k] *= if p.noise_only { 1.0 - g } else { g };
        }
        self.first = false;
    }
}

/// The streaming reducer. Without a noise print it is an exact N-sample delay line (bit-exact;
/// silence with `noise_only`), with the same latency.
///
/// Real-time: [`process`](Self::process), [`reset`](Self::reset) and
/// [`set_params`](Self::set_params) never allocate, lock or panic; everything is preallocated by
/// [`new`](Self::new).
pub struct SpectralNr {
    n: usize,
    hop: usize,
    sample_rate: f64,
    /// Last N input samples, indexed by count mod N.
    hist: Box<[f32]>,
    /// Overlap-add accumulator, indexed by output count mod N.
    ola: Box<[f32]>,
    count: u64,
    current: NrParams,
    ring: [NrParams; RING],
    stft: Option<Box<Stft>>,
}

impl SpectralNr {
    /// A reducer with FFT size `fft_size` (hop N/4) at `sample_rate`. `lambda` is the per-bin
    /// noise power ([`derive_lambda`](super::derive_lambda)); `None` = no print (delay line).
    /// Allocates the FFT plans and every buffer; starts in reset state.
    ///
    /// # Panics
    /// If `fft_size` is not a power of two ≥ 16, `sample_rate` is not finite and positive, or
    /// `lambda` does not hold `fft_size/2 + 1` values.
    pub fn new(
        fft_size: usize,
        sample_rate: f64,
        lambda: Option<Vec<f32>>,
        params: NrParams,
    ) -> Self {
        assert!(
            fft_size.is_power_of_two() && fft_size >= 16,
            "FFT size {fft_size} must be a power of two >= 16"
        );
        assert!(
            sample_rate.is_finite() && sample_rate > 0.0,
            "invalid sample rate {sample_rate}"
        );
        let stft = lambda.map(|l| {
            assert_eq!(l.len(), fft_size / 2 + 1, "lambda length");
            Box::new(Stft::new(fft_size, l))
        });
        let mut s = Self {
            n: fft_size,
            hop: fft_size / 4,
            sample_rate,
            hist: vec![0.0; fft_size].into_boxed_slice(),
            ola: vec![0.0; fft_size].into_boxed_slice(),
            count: 0,
            current: params,
            ring: [params; RING],
            stft,
        };
        s.reset();
        s
    }

    /// FFT size N.
    pub fn fft_size(&self) -> usize {
        self.n
    }

    /// Latency in samples (= N).
    pub fn latency_samples(&self) -> usize {
        self.n
    }

    /// True if a print is loaded (STFT processing), false for the delay line.
    pub fn has_print(&self) -> bool {
        self.stft.is_some()
    }

    /// The values in force from the next sample on.
    pub fn params(&self) -> NrParams {
        self.current
    }

    /// Sets the values in force from the next processed sample on. RT-safe.
    pub fn set_params(&mut self, params: NrParams) {
        self.current = params;
    }

    /// Clears history, overlap-add buffer and adaptive state; the counter restarts at 0 and the
    /// next frame snaps its smoothers. Parameter values are kept. RT-safe.
    pub fn reset(&mut self) {
        self.hist.fill(0.0);
        self.ola.fill(0.0);
        self.count = 0;
        self.ring = [self.current; RING];
        if let Some(s) = self.stft.as_mut() {
            s.clear();
        }
    }

    /// Ring slot of the snapshot that governs hop `m` (taken at boundary `(m − 4)·H`).
    fn frame_slot(m: u64) -> usize {
        ((m + 1) % RING as u64) as usize
    }

    fn boundary(&mut self, m: u64) {
        self.ring[(m % RING as u64) as usize] = self.current;
        if m == 0 {
            return;
        }
        let p = self.ring[Self::frame_slot(m)];
        let base = (self.count % self.n as u64) as usize;
        if let Some(stft) = self.stft.as_mut() {
            stft.frame(
                &self.hist,
                &mut self.ola,
                base,
                &p,
                self.sample_rate,
                self.hop,
            );
        }
    }

    /// Processes `min(input.len(), output.len())` samples. RT-safe; output depends only on the
    /// sample counts, never on the block partition.
    pub fn process(&mut self, input: &[f32], output: &mut [f32]) {
        let len = input.len().min(output.len());
        let hop = self.hop as u64;
        let n = self.n as u64;
        let mut i = 0;
        while i < len {
            let in_hop = (self.count % hop) as usize;
            let m = self.count / hop;
            if in_hop == 0 {
                self.boundary(m);
            }
            let run = (self.hop - in_hop).min(len - i);
            // N is a multiple of H, so `base + run <= N`.
            let base = (self.count % n) as usize;
            let inp = &input[i..i + run];
            let out = &mut output[i..i + run];
            let hist = &mut self.hist[base..base + run];
            if self.stft.is_some() {
                let ola = &mut self.ola[base..base + run];
                for ((o, a), (h, &x)) in out
                    .iter_mut()
                    .zip(ola.iter_mut())
                    .zip(hist.iter_mut().zip(inp))
                {
                    *o = *a;
                    *a = 0.0;
                    *h = x;
                }
            } else if self.ring[Self::frame_slot(m)].noise_only {
                for ((o, h), &x) in out.iter_mut().zip(hist.iter_mut()).zip(inp) {
                    *o = 0.0;
                    *h = x;
                }
            } else {
                for ((o, h), &x) in out.iter_mut().zip(hist.iter_mut()).zip(inp) {
                    *o = *h;
                    *h = x;
                }
            }
            i += run;
            self.count += run as u64;
        }
    }

    /// Diagnostic (tests): true if any persistent state value is subnormal.
    pub fn state_has_subnormals(&self) -> bool {
        let sub = |v: &[f32]| v.iter().any(|x| x.is_subnormal());
        sub(&self.hist)
            || sub(&self.ola)
            || self
                .stft
                .as_ref()
                .is_some_and(|s| sub(&s.p_prev) || sub(&s.g_prev) || sub(&s.g_hat))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn noise(seed: u64, n: usize, amp: f32) -> Vec<f32> {
        let mut s = seed | 1;
        (0..n)
            .map(|_| {
                s ^= s >> 12;
                s ^= s << 25;
                s ^= s >> 27;
                let u = (s.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 11) as f64 / (1u64 << 53) as f64;
                amp * (2.0 * u - 1.0) as f32
            })
            .collect()
    }

    fn run(nr: &mut SpectralNr, x: &[f32], block: usize) -> Vec<f32> {
        let mut y = vec![0.0; x.len()];
        for (i, o) in x.chunks(block).zip(y.chunks_mut(block)) {
            nr.process(i, o);
        }
        y
    }

    #[test]
    fn delay_line_is_exact() {
        let x = noise(3, 20_000, 0.5);
        for n in [1024usize, 2048] {
            let mut nr = SpectralNr::new(n, 48_000.0, None, NrParams::default());
            let y = run(&mut nr, &x, 333);
            assert!(y[..n].iter().all(|&v| v.to_bits() == 0));
            for (a, b) in y[n..].iter().zip(&x) {
                assert_eq!(a.to_bits(), b.to_bits());
            }
            let p = NrParams {
                noise_only: true,
                ..NrParams::default()
            };
            let mut nr = SpectralNr::new(n, 48_000.0, None, p);
            assert!(run(&mut nr, &x, 100).iter().all(|&v| v.to_bits() == 0));
        }
    }

    #[test]
    fn unity_gains_reconstruct_with_latency_n() {
        let x = noise(5, 30_000, 0.1);
        for n in [1024usize, 4096] {
            let lambda = vec![1e-20; n / 2 + 1];
            let mut nr = SpectralNr::new(n, 48_000.0, Some(lambda), NrParams::default());
            let y = run(&mut nr, &x, 517);
            let err = y[n..]
                .iter()
                .zip(&x)
                .map(|(a, b)| (a - b).abs())
                .fold(0.0f32, f32::max);
            assert!(err < 1e-5, "N {n}: max error {err}");
        }
    }

    #[test]
    fn block_partition_does_not_matter() {
        let x = noise(9, 40_000, 0.05);
        let lambda = vec![0.01f32 * 0.01 / 3.0 * 1024.0; 1025];
        let make = || SpectralNr::new(2048, 48_000.0, Some(lambda.clone()), NrParams::default());
        let a = run(&mut make(), &x, 4096);
        let b = run(&mut make(), &x, 1);
        let c = run(&mut make(), &x, 777);
        assert!(a.iter().zip(&b).all(|(p, q)| p.to_bits() == q.to_bits()));
        assert!(a.iter().zip(&c).all(|(p, q)| p.to_bits() == q.to_bits()));
    }

    #[test]
    fn a_change_never_reaches_output_before_its_sample_plus_n() {
        let x = noise(11, 30_000, 0.01);
        let lambda = vec![0.01f32 * 0.01 / 3.0 * 1024.0; 1025];
        let k = 10_007;
        let base = run(
            &mut SpectralNr::new(2048, 48_000.0, Some(lambda.clone()), NrParams::default()),
            &x,
            1000,
        );
        let mut nr = SpectralNr::new(2048, 48_000.0, Some(lambda), NrParams::default());
        let mut y = vec![0.0; x.len()];
        nr.process(&x[..k], &mut y[..k]);
        nr.set_params(NrParams {
            reduction_db: 30.0,
            ..NrParams::default()
        });
        nr.process(&x[k..], &mut y[k..]);
        let first = base
            .iter()
            .zip(&y)
            .position(|(a, b)| a.to_bits() != b.to_bits())
            .expect("the change has an effect");
        assert!(first >= k + 2048, "changed at {first}");
        assert!(first <= k + 2048 + 512 + 1, "changed only at {first}");
    }
}

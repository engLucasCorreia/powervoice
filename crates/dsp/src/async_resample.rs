//! Adjustable-ratio streaming resampler for the monitor path (T-107, ADR-002 §6, SPEC-002 §2.7).
//!
//! The output callback pulls the monitor ring (input-device rate) through this wrapper around
//! `rubato::Async` with septic polynomial interpolation and a fixed output side
//! (`FixedAsync::Output`): each call produces exactly the requested number of output frames
//! (1…[`MonitorResampler::MAX_CHUNK`]) and consumes a varying number of input frames —
//! [`MonitorResampler::prepare`] says how many. The ratio is `out / in` of the linked streams
//! times a drift correction clamped to ±[`MonitorResampler::MAX_CORRECTION`] and ramped over the
//! next call (`set_resample_ratio(.., ramp = true)`), so a correction never steps the pitch.
//!
//! RT use: everything is allocated in [`MonitorResampler::new`] (control thread). One instance
//! covers every rate pair within [`MonitorResampler::MAX_RATE_RATIO`] (the `Async` is built at
//! ratio 1 with that relative range), so linking an input at another rate
//! ([`MonitorResampler::set_rates`]) never allocates. `prepare`, `process`, `set_correction`,
//! `set_rates` and `reset` don't allocate, lock, log or panic (rubato's `log` feature is off).
//!
//! Polynomial interpolation has no anti-aliasing filter. Monitoring is never recorded, so that is
//! enough (ADR-002 §6); the device-rate fallback of playback and capture keep their FFT
//! resamplers ([`crate::resample`], [`crate::capture_resample`]).

use rubato::audioadapter_buffers::direct::SequentialSlice;
use rubato::{Adjustable, Async, FixedAsync, PolynomialDegree, Resampler, Resizable};

use crate::resample::ResampleError;

/// Mono, adjustable-ratio, pull-based resampler (see the module docs).
pub struct MonitorResampler {
    inner: Async<f32>,
    /// `out / in` of the linked streams.
    base_ratio: f64,
    /// Relative drift correction currently requested (1.0 = none).
    correction: f64,
    /// Input scratch for one call (`input_frames_max` long).
    input: Vec<f32>,
    /// Output frames per call currently configured in `inner`.
    chunk: usize,
}

impl std::fmt::Debug for MonitorResampler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MonitorResampler")
            .field("base_ratio", &self.base_ratio)
            .field("correction", &self.correction)
            .finish_non_exhaustive()
    }
}

impl MonitorResampler {
    /// Largest number of output frames one [`Self::process`] call produces.
    pub const MAX_CHUNK: usize = 64;
    /// Largest drift correction, relative (±1000 ppm ≈ ±1.7 cents, ADR-002 §6).
    pub const MAX_CORRECTION: f64 = 1e-3;
    /// Largest `out / in` (and `in / out`) rate ratio one instance can link.
    pub const MAX_RATE_RATIO: f64 = 8.0;
    /// Input frames the interpolator has consumed ahead of the position it outputs (half the
    /// 8-point septic kernel): once running, they are taken but not yet heard, so they count as
    /// buffered latency.
    pub const LOOKAHEAD_FRAMES: f64 = 4.0;

    /// \[control thread\] A resampler linking equal rates (call [`Self::set_rates`] to link
    /// others). Allocates its buffers once.
    ///
    /// # Errors
    /// If rubato rejects the configuration (should not happen with these constants).
    pub fn new() -> Result<Self, ResampleError> {
        let inner = Async::<f32>::new_poly(
            1.0,
            Self::MAX_RATE_RATIO * (1.0 + 2.0 * Self::MAX_CORRECTION),
            PolynomialDegree::Septic,
            Self::MAX_CHUNK,
            1,
            FixedAsync::Output,
        )
        .map_err(|e| ResampleError::Construction(e.to_string()))?;
        let input = vec![0.0; inner.input_frames_max()];
        Ok(Self {
            inner,
            base_ratio: 1.0,
            correction: 1.0,
            input,
            chunk: Self::MAX_CHUNK,
        })
    }

    /// Whether one instance can link an input at `in_hz` to an output at `out_hz`.
    pub fn supports(in_hz: u32, out_hz: u32) -> bool {
        if in_hz == 0 || out_hz == 0 {
            return false;
        }
        let r = f64::from(out_hz) / f64::from(in_hz);
        (1.0 / Self::MAX_RATE_RATIO..=Self::MAX_RATE_RATIO).contains(&r)
    }

    /// Links an input at `in_hz` to an output at `out_hz`: clears the state and sets the base
    /// ratio without a ramp (the next call starts from silence history). Returns `false` (and
    /// keeps the old link) for an unsupported pair. RT-safe.
    pub fn set_rates(&mut self, in_hz: u32, out_hz: u32) -> bool {
        if !Self::supports(in_hz, out_hz) {
            return false;
        }
        self.base_ratio = f64::from(out_hz) / f64::from(in_hz);
        self.reset();
        true
    }

    /// Clears the interpolation history and the correction (the base ratio stays). RT-safe.
    pub fn reset(&mut self) {
        self.inner.reset();
        self.chunk = Self::MAX_CHUNK;
        self.correction = 1.0;
        // In range by construction (`set_rates` checked the pair).
        let _ = self.inner.set_resample_ratio(self.base_ratio, false);
    }

    /// Requests the relative drift correction `rel` (clamped to 1 ± [`Self::MAX_CORRECTION`]);
    /// the next call ramps to it. `rel > 1` produces more output per input (the input is consumed
    /// more slowly). RT-safe.
    pub fn set_correction(&mut self, rel: f64) {
        let rel = if rel.is_finite() {
            rel.clamp(1.0 - Self::MAX_CORRECTION, 1.0 + Self::MAX_CORRECTION)
        } else {
            1.0
        };
        if rel.to_bits() != self.correction.to_bits() {
            self.correction = rel;
            let _ = self.inner.set_resample_ratio(self.base_ratio * rel, true);
        }
    }

    /// The requested relative correction.
    pub fn correction(&self) -> f64 {
        self.correction
    }

    /// `out / in` including the correction.
    pub fn ratio(&self) -> f64 {
        self.base_ratio * self.correction
    }

    /// The interpolator's own delay, in output frames (a few frames; septic = 8 points).
    pub fn delay_frames(&self) -> usize {
        self.inner.output_delay()
    }

    /// Input frames the next [`Self::process`] call consumes when it produces `out_frames`
    /// (clamped to 1…[`Self::MAX_CHUNK`]) output frames. RT-safe.
    pub fn prepare(&mut self, out_frames: usize) -> usize {
        let n = out_frames.clamp(1, Self::MAX_CHUNK);
        if n != self.chunk && self.inner.set_chunk_size(n).is_ok() {
            self.chunk = n;
        }
        self.inner.input_frames_next()
    }

    /// Produces `out.len()` output frames, which must equal the last [`Self::prepare`] argument.
    /// `fill` receives the input scratch — exactly the count `prepare` returned — and must fill
    /// all of it. Returns `false` and zeros `out` on a size mismatch (a caller bug) or a rubato
    /// error; nothing is consumed then. RT-safe.
    pub fn process(&mut self, out: &mut [f32], fill: impl FnOnce(&mut [f32])) -> bool {
        let need = self.inner.input_frames_next();
        if out.len() != self.chunk || need > self.input.len() {
            out.fill(0.0);
            return false;
        }
        fill(&mut self.input[..need]);
        let n = out.len();
        let ok = match (
            SequentialSlice::new(&self.input[..need], 1, need),
            SequentialSlice::new_mut(&mut *out, 1, n),
        ) {
            (Ok(adapter_in), Ok(mut adapter_out)) => self
                .inner
                .process_into_buffer(&adapter_in, &mut adapter_out, None)
                .is_ok(),
            _ => false,
        };
        if !ok {
            out.fill(0.0);
        }
        ok
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tone as a function of the input frame index.
    fn tone(freq_hz: f64, rate_hz: u32, k: u64) -> f32 {
        (0.5 * (std::f64::consts::TAU * freq_hz * k as f64 / f64::from(rate_hz)).sin()) as f32
    }

    /// Mean frequency from linearly interpolated rising zero crossings.
    fn frequency_hz(x: &[f32], rate_hz: u32) -> f64 {
        let mut first = None;
        let mut last = 0.0;
        let mut n = 0u64;
        for i in 1..x.len() {
            let (a, b) = (f64::from(x[i - 1]), f64::from(x[i]));
            if a < 0.0 && b >= 0.0 {
                let t = (i - 1) as f64 + a / (a - b);
                first.get_or_insert(t);
                last = t;
                n += 1;
            }
        }
        (n - 1) as f64 * f64::from(rate_hz) / (last - first.unwrap())
    }

    /// Pulls `out_len` frames in uneven pieces from a tone source; returns (output, input frames
    /// consumed).
    fn run(
        r: &mut MonitorResampler,
        rate_in: u32,
        freq_hz: f64,
        out_len: usize,
    ) -> (Vec<f32>, u64) {
        let mut out = vec![0.0f32; out_len];
        let mut k = 0u64;
        let mut done = 0;
        let mut piece = 1;
        while done < out_len {
            let n = piece.min(out_len - done).min(MonitorResampler::MAX_CHUNK);
            let need = r.prepare(n);
            assert!(r.process(&mut out[done..done + n], |buf| {
                assert_eq!(buf.len(), need);
                for x in buf.iter_mut() {
                    *x = tone(freq_hz, rate_in, k);
                    k += 1;
                }
            }));
            done += n;
            piece = piece % 61 + 7;
        }
        (out, k)
    }

    #[test]
    fn supported_pairs() {
        assert!(MonitorResampler::supports(48_000, 44_100));
        assert!(MonitorResampler::supports(8_000, 64_000));
        assert!(!MonitorResampler::supports(8_000, 96_000));
        assert!(!MonitorResampler::supports(0, 48_000));
    }

    /// A 997 Hz tone keeps its frequency across rate pairs (the base ratio is exact).
    #[test]
    fn tone_keeps_its_frequency_across_rates() {
        for (rin, rout) in [
            (48_000, 44_100),
            (44_100, 48_000),
            (48_000, 48_000),
            (96_000, 48_000),
        ] {
            let mut r = MonitorResampler::new().unwrap();
            assert!(r.set_rates(rin, rout));
            let (out, _) = run(&mut r, rin, 997.0, rout as usize * 2);
            let f = frequency_hz(&out[1_000..], rout);
            assert!((f / 997.0 - 1.0).abs() < 2e-6, "{rin}→{rout}: {f} Hz");
        }
    }

    /// The correction scales the output/input frame ratio exactly and is clamped to ±1000 ppm.
    #[test]
    fn correction_scales_the_ratio_and_is_clamped() {
        for (rel, want) in [
            (1.0005, 1.0005),
            (0.9998, 0.9998),
            (1.01, 1.001),
            (0.9, 0.999),
        ] {
            let mut r = MonitorResampler::new().unwrap();
            assert!(r.set_rates(48_000, 48_000));
            r.set_correction(rel);
            assert!((r.correction() - want).abs() < 1e-12, "{}", r.correction());
            let (_, consumed) = run(&mut r, 48_000, 440.0, 480_000);
            let measured = 480_000.0 / consumed as f64;
            // The history/lookahead at the start costs a few input frames.
            assert!((measured / want - 1.0).abs() < 3e-5, "{rel}: {measured}");
        }
    }

    /// Output frame `k` is the input signal at `k / ratio` minus a few frames of interpolator
    /// delay: an impulse comes out `delay_frames()` late at most (monitoring latency accounting).
    #[test]
    fn interpolator_delay_is_a_few_frames() {
        let mut r = MonitorResampler::new().unwrap();
        assert!(r.set_rates(48_000, 48_000));
        let mut out = vec![0.0f32; 1_000];
        let mut k = 0u64;
        for chunk in out.chunks_mut(50) {
            r.prepare(chunk.len());
            r.process(chunk, |buf| {
                for x in buf.iter_mut() {
                    *x = if k == 500 { 1.0 } else { 0.0 };
                    k += 1;
                }
            });
        }
        let peak = (0..out.len())
            .max_by(|&a, &b| out[a].abs().total_cmp(&out[b].abs()))
            .unwrap();
        assert!(
            (500..=500 + r.delay_frames()).contains(&peak),
            "peak at {peak}, delay {}",
            r.delay_frames()
        );
    }

    /// A size mismatch with `prepare` is refused without consuming input.
    #[test]
    fn mismatched_sizes_are_refused() {
        let mut r = MonitorResampler::new().unwrap();
        r.prepare(32);
        let mut out = [1.0f32; 16];
        let mut called = false;
        assert!(!r.process(&mut out, |_| called = true));
        assert!(!called);
        assert!(out.iter().all(|&x| x == 0.0));
    }
}

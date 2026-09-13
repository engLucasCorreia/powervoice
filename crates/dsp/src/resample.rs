//! Streaming fixed-ratio sample-rate conversion (ADR-002 §5, SPEC-003 §2.4): the playback reader
//! converts document rate → device rate with `rubato::Fft` (synchronous, fixed ratio). The
//! resampler is **primed**: its `output_delay()` frames are discarded after construction and
//! every [`StreamResampler::reset`], so output frame `k` is the input signal at time
//! `k / rate_out` — packet positions stay exact.
//!
//! Runs on the (non-RT) reader thread; buffers are allocated once, `pull` does not allocate.

use std::fmt;

use rubato::audioadapter_buffers::direct::SequentialSlice;
use rubato::{Fft, FixedSync, Resampler};

/// Resampler errors.
#[derive(Debug)]
pub enum ResampleError {
    /// Unsupported rates (e.g. 0 Hz).
    Construction(String),
    /// A processing call failed (should not happen with the sizes this wrapper uses).
    Process(String),
}

impl fmt::Display for ResampleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ResampleError::Construction(m) => write!(f, "resampler construction failed: {m}"),
            ResampleError::Process(m) => write!(f, "resampling failed: {m}"),
        }
    }
}

impl std::error::Error for ResampleError {}

/// Mono streaming resampler from `rate_in_hz` to `rate_out_hz`.
pub struct StreamResampler {
    inner: Fft<f32>,
    rate_in_hz: u32,
    rate_out_hz: u32,
    input: Vec<f32>,
    output: Vec<f32>,
    out_len: usize,
    out_pos: usize,
    /// Output frames still to discard (the resampler delay) after a (re)start.
    skip: usize,
}

impl fmt::Debug for StreamResampler {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StreamResampler")
            .field("rate_in_hz", &self.rate_in_hz)
            .field("rate_out_hz", &self.rate_out_hz)
            .finish_non_exhaustive()
    }
}

impl StreamResampler {
    /// Input frames per internal processing chunk.
    pub const CHUNK_IN: usize = 1024;

    /// A primed resampler.
    pub fn new(rate_in_hz: u32, rate_out_hz: u32) -> Result<Self, ResampleError> {
        let inner = Fft::<f32>::new(
            rate_in_hz as usize,
            rate_out_hz as usize,
            Self::CHUNK_IN,
            1,
            FixedSync::Input,
        )
        .map_err(|e| ResampleError::Construction(e.to_string()))?;
        let input = vec![0.0; inner.input_frames_max().max(Self::CHUNK_IN)];
        let output = vec![0.0; inner.output_frames_max()];
        let skip = inner.output_delay();
        Ok(Self {
            inner,
            rate_in_hz,
            rate_out_hz,
            input,
            output,
            out_len: 0,
            out_pos: 0,
            skip,
        })
    }

    /// Input rate.
    pub fn rate_in_hz(&self) -> u32 {
        self.rate_in_hz
    }

    /// Output rate.
    pub fn rate_out_hz(&self) -> u32 {
        self.rate_out_hz
    }

    /// Clears the internal state (seek): the next output frame is the input signal at the start
    /// of the next input chunk.
    pub fn reset(&mut self) {
        self.inner.reset();
        self.out_len = 0;
        self.out_pos = 0;
        self.skip = self.inner.output_delay();
    }

    /// Fills `out` with resampled audio. `source` is called for successive input chunks and must
    /// fill its whole argument (zero-padding past the end of the material).
    pub fn pull(
        &mut self,
        out: &mut [f32],
        mut source: impl FnMut(&mut [f32]),
    ) -> Result<(), ResampleError> {
        let mut done = 0;
        while done < out.len() {
            if self.out_pos == self.out_len {
                let need = self.inner.input_frames_next();
                source(&mut self.input[..need]);
                let cap = self.output.len();
                let written = {
                    let adapter_in = SequentialSlice::new(&self.input[..need], 1, need)
                        .map_err(|e| ResampleError::Process(e.to_string()))?;
                    let mut adapter_out = SequentialSlice::new_mut(&mut self.output[..], 1, cap)
                        .map_err(|e| ResampleError::Process(e.to_string()))?;
                    let (_, written) = self
                        .inner
                        .process_into_buffer(&adapter_in, &mut adapter_out, None)
                        .map_err(|e| ResampleError::Process(e.to_string()))?;
                    written
                };
                let skipped = self.skip.min(written);
                self.skip -= skipped;
                self.out_len = written;
                self.out_pos = skipped;
                continue;
            }
            let take = (out.len() - done).min(self.out_len - self.out_pos);
            out[done..done + take].copy_from_slice(&self.output[self.out_pos..self.out_pos + take]);
            done += take;
            self.out_pos += take;
        }
        Ok(())
    }
}

/// Offline, fixed-ratio resampling of a whole buffer in `f64` (SPEC-005 §2.12, §4.8): used by
/// export (S4-02/S4-04), not the realtime reader (which uses [`StreamResampler`] in `f32`).
///
/// Primes and trims `output_delay()` so an impulse at input sample `t` lands at
/// `round(t * rate_out_hz / rate_in_hz)` in the output, then flushes the tail with zero padding.
/// Output length is `round(input.len() * rate_out_hz / rate_in_hz)`. Equal rates are a no-op copy.
pub fn resample_offline(
    input: &[f64],
    rate_in_hz: u32,
    rate_out_hz: u32,
) -> Result<Vec<f64>, ResampleError> {
    if rate_in_hz == rate_out_hz {
        return Ok(input.to_vec());
    }
    if input.is_empty() {
        return Ok(Vec::new());
    }

    const BLOCK_IN: usize = 4096;
    let mut resampler = Fft::<f64>::new(
        rate_in_hz as usize,
        rate_out_hz as usize,
        BLOCK_IN,
        1,
        FixedSync::Input,
    )
    .map_err(|e| ResampleError::Construction(e.to_string()))?;

    let delay = resampler.output_delay();
    let expected_len =
        (input.len() as f64 * f64::from(rate_out_hz) / f64::from(rate_in_hz)).round() as usize;

    let mut in_buf = vec![0.0f64; resampler.input_frames_max().max(BLOCK_IN)];
    let out_cap = resampler.output_frames_max();
    let mut out_buf = vec![0.0f64; out_cap];
    let mut produced = Vec::with_capacity(delay + expected_len + out_cap);
    let mut pos = 0usize;

    // Feed the whole input, zero-padded past the end to flush the resampler's internal delay,
    // until enough output has been produced to cover the trimmed, expected-length result.
    while produced.len() < delay + expected_len {
        let need = resampler.input_frames_next();
        for (i, slot) in in_buf[..need].iter_mut().enumerate() {
            *slot = input.get(pos + i).copied().unwrap_or(0.0);
        }
        pos += need;
        let written = {
            let adapter_in = SequentialSlice::new(&in_buf[..need], 1, need)
                .map_err(|e| ResampleError::Process(e.to_string()))?;
            let mut adapter_out = SequentialSlice::new_mut(&mut out_buf[..], 1, out_cap)
                .map_err(|e| ResampleError::Process(e.to_string()))?;
            let (_, written) = resampler
                .process_into_buffer(&adapter_in, &mut adapter_out, None)
                .map_err(|e| ResampleError::Process(e.to_string()))?;
            written
        };
        produced.extend_from_slice(&out_buf[..written]);
        if pos >= input.len() && written == 0 {
            // Shouldn't happen with `FixedSync::Input`, but avoids spinning forever if it does;
            // the trim/pad below still yields a correctly-sized (zero-padded) result.
            break;
        }
    }

    if produced.len() > delay {
        produced.drain(0..delay);
    } else {
        produced.clear();
    }
    produced.resize(expected_len, 0.0);
    Ok(produced)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(freq_hz: f64, rate_hz: u32, start: u64, out: &mut [f32]) {
        for (i, x) in out.iter_mut().enumerate() {
            let t = (start + i as u64) as f64 / f64::from(rate_hz);
            *x = (0.5 * (std::f64::consts::TAU * freq_hz * t).sin()) as f32;
        }
    }

    /// Mean frequency from rising zero crossings (linearly interpolated).
    fn frequency_hz(x: &[f32], rate_hz: u32) -> f64 {
        let mut crossings = Vec::new();
        for i in 1..x.len() {
            let (a, b) = (f64::from(x[i - 1]), f64::from(x[i]));
            if a < 0.0 && b >= 0.0 {
                crossings.push((i - 1) as f64 + a / (a - b));
            }
        }
        let n = crossings.len() - 1;
        let span = crossings[n] - crossings[0];
        n as f64 * f64::from(rate_hz) / span
    }

    fn resample_tone(rate_in: u32, rate_out: u32, freq_hz: f64, out_len: usize) -> Vec<f32> {
        let mut r = StreamResampler::new(rate_in, rate_out).unwrap();
        let mut pos = 0u64;
        let mut out = vec![0.0f32; out_len];
        // Odd pull sizes exercise the carry-over between internal chunks.
        for chunk in out.chunks_mut(317) {
            r.pull(chunk, |buf| {
                sine(freq_hz, rate_in, pos, buf);
                pos += buf.len() as u64;
            })
            .unwrap();
        }
        out
    }

    /// SPEC-003 AC (S1-01): a 1 kHz tone at 44.1 kHz played on a 48 kHz device stays 1 kHz ± 0.05 %.
    #[test]
    fn tone_keeps_its_frequency_44k1_to_48k() {
        let out = resample_tone(44_100, 48_000, 1000.0, 48_000);
        let f = frequency_hz(&out[4_800..], 48_000);
        assert!((f - 1000.0).abs() <= 0.5, "measured {f} Hz");
    }

    /// Priming/delay compensation: output frame k is the input signal at k / rate_out.
    #[test]
    fn output_is_time_aligned_with_the_input() {
        for (rin, rout) in [(44_100, 48_000), (48_000, 44_100), (22_050, 48_000)] {
            let out = resample_tone(rin, rout, 440.0, 20_000);
            let mut expected = vec![0.0f32; out.len()];
            sine(440.0, rout, 0, &mut expected);
            let max_err = out[2_000..]
                .iter()
                .zip(&expected[2_000..])
                .map(|(a, b)| (a - b).abs())
                .fold(0.0f32, f32::max);
            assert!(max_err < 2e-3, "{rin}→{rout}: max error {max_err}");
        }
    }

    #[test]
    fn reset_restarts_aligned() {
        let mut r = StreamResampler::new(44_100, 48_000).unwrap();
        let mut junk = vec![0.0f32; 5_000];
        r.pull(&mut junk, |b| b.fill(0.3)).unwrap();
        r.reset();
        let mut pos = 0u64;
        let mut out = vec![0.0f32; 10_000];
        r.pull(&mut out, |b| {
            sine(440.0, 44_100, pos, b);
            pos += b.len() as u64;
        })
        .unwrap();
        let mut expected = vec![0.0f32; out.len()];
        sine(440.0, 48_000, 0, &mut expected);
        let max_err = out[2_000..]
            .iter()
            .zip(&expected[2_000..])
            .map(|(a, b)| (a - b).abs())
            .fold(0.0f32, f32::max);
        assert!(max_err < 2e-3, "max error {max_err}");
    }

    fn sine64(freq_hz: f64, rate_hz: u32, amp: f64, n: usize) -> Vec<f64> {
        (0..n)
            .map(|i| {
                let t = i as f64 / f64::from(rate_hz);
                amp * (std::f64::consts::TAU * freq_hz * t).sin()
            })
            .collect()
    }

    /// Mean frequency from rising zero crossings (linearly interpolated), `f64` version of the
    /// helper above.
    fn frequency_hz64(x: &[f64], rate_hz: u32) -> f64 {
        let mut crossings = Vec::new();
        for i in 1..x.len() {
            let (a, b) = (x[i - 1], x[i]);
            if a < 0.0 && b >= 0.0 {
                crossings.push((i - 1) as f64 + a / (a - b));
            }
        }
        let n = crossings.len() - 1;
        let span = crossings[n] - crossings[0];
        n as f64 * f64::from(rate_hz) / span
    }

    fn peak(x: &[f64]) -> f64 {
        x.iter().fold(0.0f64, |m, &v| m.max(v.abs()))
    }

    /// Ticket S4-02 acceptance: 48 kHz -> 44.1 kHz keeps a 1 kHz tone at 1 kHz within 0.05 % and
    /// its level within 0.1 dB (a smaller slice of SPEC-005 AC-20, which the [M6] hardening ticket
    /// checks in full).
    #[test]
    fn offline_resample_keeps_tone_frequency_and_level_48k_to_44k1() {
        let (rate_in, rate_out) = (48_000u32, 44_100u32);
        let input = sine64(1000.0, rate_in, 0.5, rate_in as usize * 2);
        let output = resample_offline(&input, rate_in, rate_out).unwrap();

        let expected_len =
            (input.len() as f64 * f64::from(rate_out) / f64::from(rate_in)).round() as usize;
        assert_eq!(output.len(), expected_len);

        // Measure on the back half only, clear of the flushed/padded tail and any startup ripple.
        let tail = &output[output.len() / 2..(output.len() - 200)];
        let f = frequency_hz64(tail, rate_out);
        let rel_err = (f - 1000.0).abs() / 1000.0;
        assert!(
            rel_err <= 0.0005,
            "measured {f} Hz ({}% off)",
            rel_err * 100.0
        );

        let in_tail = &input[input.len() / 2..];
        let level_db = 20.0 * (peak(tail) / peak(in_tail)).log10();
        assert!(level_db.abs() <= 0.1, "level changed by {level_db} dB");
    }

    /// Equal rates are a bit-exact no-op (no resampler constructed).
    #[test]
    fn offline_resample_is_a_no_op_for_equal_rates() {
        let input = sine64(440.0, 48_000, 0.3, 1_000);
        let output = resample_offline(&input, 48_000, 48_000).unwrap();
        assert_eq!(input, output);
    }

    /// Output length tracks the input/output ratio exactly (SPEC-005 §2.12 "Output length =
    /// round(len x fs_out/fs_in) +/- 1"); here the offline resampler hits it exactly.
    #[test]
    fn offline_resample_output_length_matches_the_ratio() {
        for (rate_in, rate_out) in [(48_000u32, 44_100u32), (44_100, 48_000), (48_000, 96_000)] {
            let input = sine64(200.0, rate_in, 0.2, rate_in as usize);
            let output = resample_offline(&input, rate_in, rate_out).unwrap();
            let expected =
                (input.len() as f64 * f64::from(rate_out) / f64::from(rate_in)).round() as usize;
            assert_eq!(output.len(), expected, "{rate_in} -> {rate_out}");
        }
    }

    #[test]
    fn offline_resample_of_empty_input_is_empty() {
        let output = resample_offline(&[], 48_000, 44_100).unwrap();
        assert!(output.is_empty());
    }
}

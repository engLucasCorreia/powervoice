//! Streaming, push-based resampling for the capture-writer thread (H-06, SPEC-002 §2.2 "If the
//! input device can't run at the document rate"): the writer feeds device-rate samples as they
//! drain from the capture ring and gets document-rate samples back to append to the take.
//!
//! Unlike [`crate::resample::StreamResampler`] (built for the playback reader, which can always
//! read ahead into the document and so can zero-pad past the end whenever it needs more input),
//! this type never asks for data that hasn't arrived yet: [`CaptureResampler::push`] buffers a
//! partial chunk internally and only resamples once a full [`CaptureResampler::CHUNK_IN`] block
//! has arrived, so nothing is ever fabricated mid-take. [`CaptureResampler::finish`] flushes the
//! last partial chunk (zero-padded) once, at the end of the take — the same trailing-flush
//! technique as [`crate::resample::resample_offline`].
//!
//! Primed like `StreamResampler`: [`rubato::Resampler::output_delay`] frames are discarded at the
//! front, so the take's first written sample lines up with the input's first captured sample —
//! the capture-writer needs no extra bookkeeping to shift the take's start time for the
//! resampler's own latency (ticket H-06 "latency accounted in the take start").
//!
//! Runs on the capture-writer thread only, never the RT input callback (CLAUDE.md real-time
//! rules): buffers are allocated once, at construction; `push` and `finish` never allocate other
//! than growing the caller's `out: &mut Vec<f32>` (already the convention `TakeCapture::append`'s
//! callers use for scratch buffers off the audio thread).

use rubato::audioadapter_buffers::direct::SequentialSlice;
use rubato::{Fft, FixedSync, Resampler};

use crate::resample::ResampleError;

/// Converts one mono stream from `rate_in_hz` to `rate_out_hz` as blocks of samples arrive.
pub struct CaptureResampler {
    inner: Fft<f32>,
    rate_in_hz: u32,
    rate_out_hz: u32,
    /// Input frames per internal processing chunk (`inner.input_frames_next()`, fixed since the
    /// resampler is built with `FixedSync::Input`).
    chunk_in: usize,
    /// Buffered input not yet enough for a full chunk (`input[..fill]`).
    input: Vec<f32>,
    fill: usize,
    /// Scratch for one `process_into_buffer` call.
    output: Vec<f32>,
    /// Output frames still to discard (the resampler's own delay).
    skip: usize,
}

impl std::fmt::Debug for CaptureResampler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CaptureResampler")
            .field("rate_in_hz", &self.rate_in_hz)
            .field("rate_out_hz", &self.rate_out_hz)
            .finish_non_exhaustive()
    }
}

impl CaptureResampler {
    /// Input frames per internal processing chunk (same as
    /// [`crate::resample::StreamResampler::CHUNK_IN`]).
    pub const CHUNK_IN: usize = 1024;

    /// A primed resampler converting `rate_in_hz` to `rate_out_hz`.
    pub fn new(rate_in_hz: u32, rate_out_hz: u32) -> Result<Self, ResampleError> {
        let inner = Fft::<f32>::new(
            rate_in_hz as usize,
            rate_out_hz as usize,
            Self::CHUNK_IN,
            1,
            FixedSync::Input,
        )
        .map_err(|e| ResampleError::Construction(e.to_string()))?;
        let chunk_in = inner.input_frames_next().max(1);
        let input = vec![0.0; chunk_in];
        let output = vec![0.0; inner.output_frames_max()];
        let skip = inner.output_delay();
        Ok(Self {
            inner,
            rate_in_hz,
            rate_out_hz,
            chunk_in,
            input,
            fill: 0,
            output,
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

    /// Runs one full `chunk_in`-sized buffer through the resampler and appends whatever isn't
    /// still being discarded as priming delay to `out`. A processing failure (should not happen
    /// with the fixed sizes this wrapper uses) yields silence for that chunk rather than
    /// panicking or losing track of position — the same convention `reader.rs` uses for
    /// `StreamResampler::pull` failures.
    fn process_chunk(&mut self, out: &mut Vec<f32>) {
        let cap = self.output.len();
        let written = (|| -> Result<usize, ResampleError> {
            let adapter_in = SequentialSlice::new(&self.input[..self.chunk_in], 1, self.chunk_in)
                .map_err(|e| ResampleError::Process(e.to_string()))?;
            let mut adapter_out = SequentialSlice::new_mut(&mut self.output[..], 1, cap)
                .map_err(|e| ResampleError::Process(e.to_string()))?;
            let (_, written) = self
                .inner
                .process_into_buffer(&adapter_in, &mut adapter_out, None)
                .map_err(|e| ResampleError::Process(e.to_string()))?;
            Ok(written)
        })()
        .unwrap_or(0);
        let skip = self.skip.min(written);
        self.skip -= skip;
        out.extend_from_slice(&self.output[skip..written]);
    }

    /// Feeds device-rate `samples`, appending every document-rate sample they produce to `out`.
    /// Buffers any tail shorter than a full chunk for the next call or [`Self::finish`].
    pub fn push(&mut self, mut samples: &[f32], out: &mut Vec<f32>) {
        while !samples.is_empty() {
            let need = self.chunk_in - self.fill;
            let take = need.min(samples.len());
            self.input[self.fill..self.fill + take].copy_from_slice(&samples[..take]);
            self.fill += take;
            samples = &samples[take..];
            if self.fill == self.chunk_in {
                self.process_chunk(out);
                self.fill = 0;
            }
        }
    }

    /// Flushes the take's tail: zero-pads any buffered partial chunk through the resampler (like
    /// `resample_offline`'s trailing flush) and appends the result to `out`. Consumes `self` —
    /// call once, when the take ends.
    pub fn finish(mut self, out: &mut Vec<f32>) {
        if self.fill > 0 {
            self.input[self.fill..self.chunk_in].fill(0.0);
            self.process_chunk(out);
            self.fill = 0;
        }
    }
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

    fn peak(x: &[f32]) -> f64 {
        x.iter().fold(0.0f64, |m, &v| m.max(f64::from(v.abs())))
    }

    /// Runs a whole tone through the resampler in one `push` + `finish`.
    /// `finish()`'s zero-padded flush produces up to one chunk's worth of output past the "real"
    /// signal (`expected_len`, the same quantity `resample_offline` trims to exactly) — trim it
    /// here too so measurements never read the flush's zero-input tail.
    fn resample_whole(rate_in: u32, rate_out: u32, freq_hz: f64, secs: f64) -> (Vec<f32>, usize) {
        let n_in = (f64::from(rate_in) * secs) as usize;
        let mut input = vec![0.0f32; n_in];
        sine(freq_hz, rate_in, 0, &mut input);
        let mut r = CaptureResampler::new(rate_in, rate_out).unwrap();
        let mut out = Vec::new();
        r.push(&input, &mut out);
        r.finish(&mut out);
        let expected_len =
            (n_in as f64 * f64::from(rate_out) / f64::from(rate_in)).round() as usize;
        out.truncate(expected_len.min(out.len()));
        (out, n_in)
    }

    /// H-06 ticket AC: 48 kHz input into a 44.1 kHz document keeps a 1 kHz tone within 0.05 % and
    /// its level within 0.1 dB.
    #[test]
    fn tone_keeps_frequency_and_level_48k_to_44k1() {
        let (out, n_in) = resample_whole(48_000, 44_100, 1_000.0, 2.0);
        // Measure on the back half, clear of the priming edge.
        let tail = &out[out.len() / 2..];
        let f = frequency_hz(tail, 44_100);
        let rel_err = (f - 1_000.0).abs() / 1_000.0;
        assert!(
            rel_err <= 0.0005,
            "measured {f} Hz ({}% off)",
            rel_err * 100.0
        );

        let mut in_tail = vec![0.0f32; n_in / 2];
        sine(1_000.0, 48_000, (n_in / 2) as u64, &mut in_tail);
        let level_db = 20.0 * (peak(tail) / peak(&in_tail)).log10();
        assert!(level_db.abs() <= 0.1, "level changed by {level_db} dB");
    }

    /// The reverse direction (44.1 kHz input, 48 kHz document) from the ticket's own example.
    #[test]
    fn tone_keeps_frequency_and_level_44k1_to_48k() {
        let (out, n_in) = resample_whole(44_100, 48_000, 1_000.0, 2.0);
        let tail = &out[out.len() / 2..];
        let f = frequency_hz(tail, 48_000);
        let rel_err = (f - 1_000.0).abs() / 1_000.0;
        assert!(
            rel_err <= 0.0005,
            "measured {f} Hz ({}% off)",
            rel_err * 100.0
        );

        let mut in_tail = vec![0.0f32; n_in / 2];
        sine(1_000.0, 44_100, (n_in / 2) as u64, &mut in_tail);
        let level_db = 20.0 * (peak(tail) / peak(&in_tail)).log10();
        assert!(level_db.abs() <= 0.1, "level changed by {level_db} dB");
    }

    /// The take length matches wall time within one processing block either way (ticket AC).
    #[test]
    fn output_length_matches_wall_time_within_one_block() {
        for (rate_in, rate_out) in [(48_000u32, 44_100u32), (44_100, 48_000), (44_100, 96_000)] {
            let (out, n_in) = resample_whole(rate_in, rate_out, 440.0, 10.0);
            let expected =
                (n_in as f64 * f64::from(rate_out) / f64::from(rate_in)).round() as usize;
            let block_out = (CaptureResampler::CHUNK_IN as f64 * f64::from(rate_out)
                / f64::from(rate_in))
            .ceil() as usize;
            assert!(
                out.len().abs_diff(expected) <= block_out,
                "{rate_in} -> {rate_out}: got {}, expected {expected} +/- {block_out}",
                out.len()
            );
        }
    }

    /// Feeding the ring's drained chunks in small, uneven pieces (as the capture-writer really
    /// does every ~20 ms) gives the exact same result as one big push — incremental buffering
    /// doesn't shift or drop samples at chunk boundaries.
    #[test]
    fn uneven_incremental_pushes_match_one_big_push() {
        let n = 48_000 * 3;
        let mut input = vec![0.0f32; n];
        sine(440.0, 48_000, 0, &mut input);

        let mut whole = CaptureResampler::new(48_000, 44_100).unwrap();
        let mut out_whole = Vec::new();
        whole.push(&input, &mut out_whole);
        whole.finish(&mut out_whole);

        let mut piecewise = CaptureResampler::new(48_000, 44_100).unwrap();
        let mut out_pieces = Vec::new();
        // Odd, ring-drain-like chunk sizes.
        for chunk in input.chunks(937) {
            piecewise.push(chunk, &mut out_pieces);
        }
        piecewise.finish(&mut out_pieces);

        assert_eq!(out_whole, out_pieces);
    }

    /// A take shorter than one internal chunk still flushes cleanly (no panic, no division by
    /// zero) instead of losing the whole take.
    #[test]
    fn a_take_shorter_than_one_chunk_still_flushes() {
        let mut r = CaptureResampler::new(48_000, 44_100).unwrap();
        let mut out = Vec::new();
        let short = vec![0.1f32; 200];
        r.push(&short, &mut out);
        r.finish(&mut out);
        // Some output may be entirely consumed by the priming delay — that's fine; the point is
        // it doesn't panic and doesn't produce more than one chunk's worth.
        assert!(out.len() <= CaptureResampler::CHUNK_IN * 2);
    }

    /// Priming/delay compensation, mirroring `StreamResampler`'s equivalent test: output frame k
    /// is the input signal at k / rate_out, i.e. the take's start isn't shifted by the
    /// resampler's own latency (H-06 "latency accounted in the take start").
    #[test]
    fn output_is_time_aligned_with_the_input() {
        let (out, _) = resample_whole(44_100, 48_000, 440.0, 0.5);
        let mut expected = vec![0.0f32; out.len()];
        sine(440.0, 48_000, 0, &mut expected);
        let max_err = out[2_000..]
            .iter()
            .zip(&expected[2_000..])
            .map(|(a, b)| (a - b).abs())
            .fold(0.0f32, f32::max);
        assert!(max_err < 2e-3, "max error {max_err}");
    }
}

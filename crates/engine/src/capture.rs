//! The capture-writer (ADR-002 §1, ADR-004 §7, SPEC-002 §2.3, §4.1).
//!
//! Drains the capture ring into the take — crash-safe WAV first, then the chunk store — at least
//! every [`DRAIN_INTERVAL`] (the 250 ms process-crash bound needs ≤ 50 ms), and on a second thread
//! patches + `fdatasync`s the WAV header about every second through the `TakeSyncHandle` (the
//! ~1.5 s power-loss bound; T-101 handoff: `TakeSyncMode::Background` never syncs in `append`).
//! When the take is complete — the input callback pushed its last sample, the input stream is
//! gone, or the control thread forces it — the writer calls `TakeCapture::finish()`, hands the
//! capture-ring consumer back for the next take and calls the [`RecordDone`] callback.
//!
//! `ManualEngine` runs the same [`CaptureWriter`] inline from its tick (deterministic tests).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use rtrb::Consumer;
use vox_project::take::DEFAULT_TAKE_SYNC_INTERVAL;
use vox_project::{ProjectError, TakeCapture};

use crate::input::{InputShared, stop_code};
use crate::record::{LIVE_PEAKS_SPB, RecordDone, RecordingResult, StopReason};

/// Drain period of the writer thread (≤ 50 ms, SPEC-002 §4.1).
pub(crate) const DRAIN_INTERVAL: Duration = Duration::from_millis(20);

/// Where the writer returns the capture-ring consumer when a take is finished.
pub(crate) type CaptureHome = Arc<Mutex<Option<Consumer<f32>>>>;

/// Running min/max peaks for the take being captured (H-07, bucket size [`LIVE_PEAKS_SPB`]):
/// appended to only by the capture-writer thread as it drains the capture ring — never by the RT
/// input callback. Read by the control thread (`EngineHandle::live_take_peaks`) through
/// [`LivePeaksHandle`]'s lock, a lock the RT thread never touches.
pub(crate) struct LivePeaks {
    /// Completed buckets.
    buckets: Vec<(f32, f32)>,
    /// The in-progress bucket: running min/max and sample count.
    partial: (f32, f32),
    partial_len: u32,
    /// Samples appended so far.
    len_samples: u64,
}

impl Default for LivePeaks {
    fn default() -> Self {
        Self {
            buckets: Vec::new(),
            partial: (f32::INFINITY, f32::NEG_INFINITY),
            partial_len: 0,
            len_samples: 0,
        }
    }
}

/// Shared between the capture-writer thread (the only writer) and the control thread (the only
/// reader).
pub(crate) type LivePeaksHandle = Arc<Mutex<LivePeaks>>;

impl LivePeaks {
    /// Appends samples just written to the take (allocates only here, on the writer thread).
    fn append(&mut self, samples: &[f32]) {
        for &s in samples {
            self.partial.0 = self.partial.0.min(s);
            self.partial.1 = self.partial.1.max(s);
            self.partial_len += 1;
            if self.partial_len == LIVE_PEAKS_SPB {
                self.buckets.push(self.partial);
                self.partial = (f32::INFINITY, f32::NEG_INFINITY);
                self.partial_len = 0;
            }
        }
        self.len_samples += samples.len() as u64;
    }

    /// `(len_samples, buckets[start_bucket..start_bucket + max])`, clamped; the in-progress
    /// bucket counts as one more (non-empty) bucket at the end.
    pub(crate) fn snapshot(&self, start_bucket: u32, max: u32) -> (u64, Vec<(f32, f32)>) {
        let total = self.buckets.len() + usize::from(self.partial_len > 0);
        let start = (start_bucket as usize).min(total);
        let end = start.saturating_add(max as usize).min(total);
        let buckets = (start..end)
            .map(|i| self.buckets.get(i).copied().unwrap_or(self.partial))
            .collect();
        (self.len_samples, buckets)
    }
}

/// \[control thread\] Takes the consumer a finished writer returned.
pub(crate) fn take_home(home: &CaptureHome) -> Option<Consumer<f32>> {
    home.lock().unwrap_or_else(PoisonError::into_inner).take()
}

/// One take's writer state.
pub(crate) struct CaptureWriter {
    rx: Consumer<f32>,
    capture: TakeCapture,
    shared: Arc<InputShared>,
    home: CaptureHome,
    rate_hz: u32,
    write_error: Option<ProjectError>,
    done: Option<RecordDone>,
    /// H-07: running min/max peaks of the samples appended to `capture` so far.
    peaks: LivePeaksHandle,
}

impl CaptureWriter {
    pub(crate) fn new(
        rx: Consumer<f32>,
        capture: TakeCapture,
        shared: Arc<InputShared>,
        home: CaptureHome,
        rate_hz: u32,
        done: RecordDone,
        peaks: LivePeaksHandle,
    ) -> Self {
        Self {
            rx,
            capture,
            shared,
            home,
            rate_hz,
            write_error: None,
            done: Some(done),
            peaks,
        }
    }

    /// Appends everything in the ring to the take (and its H-07 live peaks). Returns `true` once
    /// the take is complete and the ring is empty. After an append error the rest of the take is
    /// drained and dropped, and [`InputShared::writer_failed`] asks the control thread to stop
    /// the recording (H-05: the take is kept up to the last good sample, SPEC-002 §2.5).
    pub(crate) fn drain(&mut self) -> bool {
        // Read the completion flags first: every sample pushed before they were set is then
        // visible to the drain below (Release/Acquire).
        let complete = self.shared.capture_done.load(Ordering::Acquire)
            || self.shared.force_finish.load(Ordering::Acquire)
            || self.rx.is_abandoned();
        let n = self.rx.slots();
        if n > 0
            && let Ok(chunk) = self.rx.read_chunk(n)
        {
            let (a, b) = chunk.as_slices();
            if self.write_error.is_none() {
                for part in [a, b] {
                    if part.is_empty() {
                        continue;
                    }
                    match self.capture.append(part) {
                        Ok(()) => self
                            .peaks
                            .lock()
                            .unwrap_or_else(PoisonError::into_inner)
                            .append(part),
                        Err(e) => {
                            self.write_error = Some(e);
                            self.shared.writer_failed.store(true, Ordering::Release);
                            break;
                        }
                    }
                }
            }
            chunk.commit_all();
        }
        complete && self.rx.is_empty()
    }

    /// Patches the WAV header + `fdatasync`s on this thread (inline mode).
    pub(crate) fn sync(&mut self) {
        let _ = self.capture.sync();
    }

    /// Finishes the take, returns the ring consumer home, then calls the done callback.
    pub(crate) fn finish(mut self) {
        let reason = match self.shared.stop_reason.load(Ordering::Relaxed) {
            stop_code::INPUT_LOST => StopReason::InputLost,
            stop_code::SHUTDOWN => StopReason::Shutdown,
            stop_code::OVERFLOW => StopReason::Overflow,
            stop_code::WRITE_ERROR => StopReason::WriteError,
            _ => StopReason::User,
        };
        let finished = self.capture.finish();
        *self.home.lock().unwrap_or_else(PoisonError::into_inner) = Some(self.rx);
        let result = RecordingResult {
            finished,
            reason,
            sample_rate_hz: self.rate_hz,
            clip_events: self.shared.clip_events.load(Ordering::Relaxed),
            overflow_samples: self.shared.overflow_samples.load(Ordering::Relaxed),
            write_error: self.write_error.take(),
        };
        if let Some(done) = self.done.take() {
            done(result);
        }
    }
}

/// Runs `writer` on its own thread (plus the ~1 s sync thread) until the take is finished.
pub(crate) fn spawn(mut writer: CaptureWriter) -> std::io::Result<JoinHandle<()>> {
    let sync = writer.capture.sync_handle();
    thread::Builder::new()
        .name("vox-capture".into())
        .spawn(move || {
            let stop = Arc::new(AtomicBool::new(false));
            let stop_sync = stop.clone();
            let syncer = thread::Builder::new()
                .name("vox-take-sync".into())
                .spawn(move || {
                    loop {
                        thread::park_timeout(DEFAULT_TAKE_SYNC_INTERVAL);
                        if stop_sync.load(Ordering::Acquire) {
                            break;
                        }
                        // A failed sync is retried a second later; `finish` reports errors.
                        let _ = sync.sync();
                    }
                })
                .ok();
            while !writer.drain() {
                thread::sleep(DRAIN_INTERVAL);
            }
            stop.store(true, Ordering::Release);
            if let Some(s) = syncer {
                s.thread().unpark();
                let _ = s.join();
            }
            writer.finish();
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Deterministic pseudo-random samples in `[-0.5, 0.5)` (same construction as
    /// `crates/engine/tests/recording.rs`'s `src`, kept local to avoid a new dev-dependency).
    fn synth(i: u64) -> f32 {
        let mut z = i.wrapping_mul(0x9E37_79B9_7F4A_7C15);
        z ^= z >> 31;
        z = z.wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z ^= z >> 29;
        (z >> 40) as f32 / (1u64 << 24) as f32 - 0.5
    }

    fn brute_force(samples: &[f32], spb: usize) -> Vec<(f32, f32)> {
        samples
            .chunks(spb)
            .map(|c| {
                c.iter()
                    .fold((f32::INFINITY, f32::NEG_INFINITY), |(mn, mx), &s| {
                        (mn.min(s), mx.max(s))
                    })
            })
            .collect()
    }

    /// H-07: the running peaks match brute-force min/max of every sample appended, including a
    /// partial last bucket, across several uneven `append()` calls — exactly how `drain()` sees
    /// uneven ring reads.
    #[test]
    fn live_peaks_match_brute_force_across_uneven_appends() {
        let samples: Vec<f32> = (0..5_003).map(synth).collect();
        let mut peaks = LivePeaks::default();
        for chunk in samples.chunks(37) {
            peaks.append(chunk);
        }
        assert_eq!(peaks.len_samples, samples.len() as u64);
        let (len, got) = peaks.snapshot(0, u32::MAX);
        assert_eq!(len, samples.len() as u64);
        let want = brute_force(&samples, LIVE_PEAKS_SPB as usize);
        assert_eq!(got, want);
    }

    /// `snapshot` pages by bucket index and clamps a `start_bucket`/`max` past the end.
    #[test]
    fn snapshot_pages_and_clamps_start_bucket() {
        let samples: Vec<f32> = (0..u64::from(LIVE_PEAKS_SPB * 5) + 10).map(synth).collect();
        let mut peaks = LivePeaks::default();
        peaks.append(&samples);
        let (_, all) = peaks.snapshot(0, 100);
        assert_eq!(all.len(), 6, "5 full buckets + 1 partial");
        let (_, page) = peaks.snapshot(2, 2);
        assert_eq!(page, all[2..4]);
        let (_, past_end) = peaks.snapshot(50, 10);
        assert!(past_end.is_empty());
    }

    /// An empty take reports zero length and no buckets (no divide-by-zero / underflow on the
    /// first `drain()` before anything was appended).
    #[test]
    fn empty_live_peaks_snapshot_is_empty() {
        let peaks = LivePeaks::default();
        let (len, buckets) = peaks.snapshot(0, 10);
        assert_eq!((len, buckets.len()), (0, 0));
    }
}

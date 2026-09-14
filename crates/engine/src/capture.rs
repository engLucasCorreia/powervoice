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
//! T-304 (SPEC-022 §4.5): during a record operation the take holds the whole pass (pre-roll,
//! window, post-roll). The control thread decides the record window and **seals** the operation
//! ([`OpShared`]); the writer finishes only after that, so the result always carries the decided
//! window. Live peaks describe the window only (take index `k_start` on): samples appended before
//! `k_start` became known are kept in a 2 s look-back and replayed.
//!
//! `ManualEngine` runs the same [`CaptureWriter`] inline from its tick (deterministic tests).

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use rtrb::Consumer;
use vox_dsp::capture_resample::CaptureResampler;
use vox_project::take::DEFAULT_TAKE_SYNC_INTERVAL;
use vox_project::{ProjectError, TakeCapture};

use crate::input::{GapEvent, InputShared, UNKNOWN_GAP, stop_code};
use crate::record::{
    DropoutMark, LIVE_PEAKS_SPB, OpResult, RecordDone, RecordingResult, StopReason,
};

/// Drain period of the writer thread (≤ 50 ms, SPEC-002 §4.1).
pub(crate) const DRAIN_INTERVAL: Duration = Duration::from_millis(20);
/// SPEC-022 §3 `capture_history_s`: look-back kept for a window start that lies in the past.
const LOOKBACK_S: usize = 2;

/// Where the writer returns the capture-ring consumer when a take is finished.
pub(crate) type CaptureHome = Arc<Mutex<Option<Consumer<f32>>>>;
/// Where the writer returns the gap-ring consumer when a take is finished (H-10 item 4, mirrors
/// [`CaptureHome`]).
pub(crate) type GapHome = Arc<Mutex<Option<Consumer<GapEvent>>>>;

/// T-304 (SPEC-022 §4.5): a record operation's state shared by the control thread (which
/// decides the window) and the capture-writer (which finishes the take with it). Never touched
/// by an RT callback.
#[derive(Debug)]
pub(crate) struct OpShared {
    /// The record window's first take sample (document rate); `u64::MAX` until known.
    pub(crate) k_start: AtomicU64,
    /// Set (Release) once `outcome` holds the decision: only then may the writer finish.
    sealed: AtomicBool,
    outcome: Mutex<Option<OpResult>>,
}

impl OpShared {
    pub(crate) fn new() -> Self {
        Self {
            k_start: AtomicU64::new(u64::MAX),
            sealed: AtomicBool::new(false),
            outcome: Mutex::new(None),
        }
    }

    /// \[control thread\] Records the decision; the writer may now finish the take.
    pub(crate) fn seal(&self, result: OpResult) {
        *self.outcome.lock().unwrap_or_else(PoisonError::into_inner) = Some(result);
        self.sealed.store(true, Ordering::Release);
    }

    pub(crate) fn is_sealed(&self) -> bool {
        self.sealed.load(Ordering::Acquire)
    }

    fn take_outcome(&self) -> Option<OpResult> {
        self.outcome
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
    }
}

/// \[control thread\] Takes the gap-ring consumer a finished writer returned.
pub(crate) fn take_gap_home(home: &GapHome) -> Option<Consumer<GapEvent>> {
    home.lock().unwrap_or_else(PoisonError::into_inner).take()
}

/// H-10 item 6: once completed buckets reach this count, [`LivePeaks::decimate`] halves the
/// array and doubles the effective samples-per-bucket, so memory stays bounded (≈ `8 ×
/// MAX_LIVE_PEAK_BUCKETS` bytes ≈ 512 KiB) whatever the take's length — at 48 kHz an
/// undecimated take would otherwise grow the buckets `Vec` by ~5.4 MB/h forever. Same cap as
/// `record_commands::MAX_LIVE_PEAKS_BUCKETS`, so one request can still return the whole take.
const MAX_LIVE_PEAK_BUCKETS: usize = 65_536;

/// Running min/max peaks for the take being captured (H-07, starting bucket size
/// [`LIVE_PEAKS_SPB`], H-10 item 6 doubles it as the take grows past
/// [`MAX_LIVE_PEAK_BUCKETS`]): appended to only by the capture-writer thread as it drains the
/// capture ring — never by the RT input callback. Read by the control thread
/// (`EngineHandle::live_take_peaks`) through [`LivePeaksHandle`]'s lock, a lock the RT thread
/// never touches.
pub(crate) struct LivePeaks {
    /// Completed buckets, each covering `spb` samples.
    buckets: Vec<(f32, f32)>,
    /// The in-progress bucket: running min/max and sample count.
    partial: (f32, f32),
    partial_len: u32,
    /// Samples appended so far.
    len_samples: u64,
    /// Current samples per completed/in-progress bucket — starts at [`LIVE_PEAKS_SPB`] and
    /// doubles every time [`Self::decimate`] runs.
    spb: u32,
}

impl Default for LivePeaks {
    fn default() -> Self {
        Self {
            buckets: Vec::new(),
            partial: (f32::INFINITY, f32::NEG_INFINITY),
            partial_len: 0,
            len_samples: 0,
            spb: LIVE_PEAKS_SPB,
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
            if self.partial_len == self.spb {
                self.buckets.push(self.partial);
                self.partial = (f32::INFINITY, f32::NEG_INFINITY);
                self.partial_len = 0;
                if self.buckets.len() >= MAX_LIVE_PEAK_BUCKETS {
                    self.decimate();
                }
            }
        }
        self.len_samples += samples.len() as u64;
    }

    /// Halves `buckets` by merging every adjacent pair (combined min/max) and doubles `spb` —
    /// bounded, allocating work, run only here on the capture-writer thread (never the RT
    /// callback). A leftover unpaired bucket (not reachable at a power-of-two cap, but handled
    /// regardless) carries over unmerged.
    fn decimate(&mut self) {
        let (chunks, remainder) = self.buckets.as_chunks::<2>();
        let mut merged: Vec<(f32, f32)> = chunks
            .iter()
            .map(|pair| (pair[0].0.min(pair[1].0), pair[0].1.max(pair[1].1)))
            .collect();
        merged.extend_from_slice(remainder);
        self.buckets = merged;
        self.spb = self.spb.saturating_mul(2);
    }

    /// `(len_samples, spb, buckets[start_bucket..start_bucket + max])`, clamped; the in-progress
    /// bucket counts as one more (non-empty) bucket at the end. `spb` is the bucket size *this*
    /// snapshot's buckets use — it can grow between polls as the take decimates.
    pub(crate) fn snapshot(&self, start_bucket: u32, max: u32) -> (u64, u32, Vec<(f32, f32)>) {
        let total = self.buckets.len() + usize::from(self.partial_len > 0);
        let start = (start_bucket as usize).min(total);
        let end = start.saturating_add(max as usize).min(total);
        let buckets = (start..end)
            .map(|i| self.buckets.get(i).copied().unwrap_or(self.partial))
            .collect();
        (self.len_samples, self.spb, buckets)
    }
}

/// \[control thread\] Takes the consumer a finished writer returned.
pub(crate) fn take_home(home: &CaptureHome) -> Option<Consumer<f32>> {
    home.lock().unwrap_or_else(PoisonError::into_inner).take()
}

/// H-10 item 4: samples of silence appended per [`CaptureWriter::append_silence`] call — bounded,
/// stack-allocated, so a large dropout fills in a loop rather than needing one big buffer.
const SILENCE_CHUNK: usize = 1024;

/// One take's writer state.
pub(crate) struct CaptureWriter {
    rx: Consumer<f32>,
    capture: TakeCapture,
    shared: Arc<InputShared>,
    home: CaptureHome,
    /// The take's rate (= the document rate `capture` was created at).
    rate_hz: u32,
    /// `Some` when the input stream runs at a different rate than the take (H-06, SPEC-002 §2.2
    /// "If the input device can't run at the document rate"): converts device-rate samples
    /// drained from `rx` to `rate_hz` before they reach `capture`. Never touched by the RT input
    /// callback — only this (non-RT) writer thread runs it.
    resampler: Option<CaptureResampler>,
    /// Reused scratch for one resampler `push`/`finish` call (grows once, then stays warm).
    resample_scratch: Vec<f32>,
    /// Reused scratch holding one `read_chunk`'s two slices concatenated (needed so `chunk` — a
    /// borrow of `rx` — can be committed and dropped before `drain_part` borrows `self` as a
    /// whole; grows once, then stays warm).
    drain_scratch: Vec<f32>,
    write_error: Option<ProjectError>,
    done: Option<RecordDone>,
    /// H-07: running min/max peaks of the samples appended to `capture` so far.
    peaks: LivePeaksHandle,
    /// H-10 item 4 / H-11 item 5 (SPEC-002 §2.4/§4.3): where the RT input callback reports
    /// dropouts, in the same device-rate ring-position numbering as [`Self::rx`] — regardless of
    /// whether [`Self::resampler`] is active (the silence fill is spliced in before resampling,
    /// `splice_gaps_and_append`). `None` only when the ring wasn't available for this take —
    /// dropouts are then simply not detected.
    gap_rx: Option<Consumer<GapEvent>>,
    gap_home: GapHome,
    /// Raw (device-rate) ring samples consumed so far — the same numbering as `GapEvent::take_index`.
    ring_consumed: u64,
    /// Gap events received but not yet reached by `ring_consumed`.
    pending_gaps: VecDeque<GapEvent>,
    /// Dropouts filled so far, to mark once the take is committed (`RecordingResult::dropouts`).
    dropouts: Vec<DropoutMark>,
    /// T-304: the record operation this take belongs to (`None`: a new recording).
    op: Option<Arc<OpShared>>,
    /// Take samples (document rate) appended so far.
    written: u64,
    /// Live peaks describe take samples from here on (the record window); `None` until the
    /// operation's window start is known.
    peaks_from: Option<u64>,
    /// The last ≤ [`LOOKBACK_S`] of appended samples while `peaks_from` is unknown.
    lookback: VecDeque<f32>,
    lookback_cap: usize,
}

impl CaptureWriter {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        rx: Consumer<f32>,
        capture: TakeCapture,
        shared: Arc<InputShared>,
        home: CaptureHome,
        rate_hz: u32,
        resampler: Option<CaptureResampler>,
        done: RecordDone,
        peaks: LivePeaksHandle,
        gap_rx: Option<Consumer<GapEvent>>,
        gap_home: GapHome,
        op: Option<Arc<OpShared>>,
    ) -> Self {
        Self {
            rx,
            capture,
            shared,
            home,
            rate_hz,
            resampler,
            resample_scratch: Vec::new(),
            drain_scratch: Vec::new(),
            write_error: None,
            done: Some(done),
            peaks,
            gap_rx,
            gap_home,
            ring_consumed: 0,
            pending_gaps: VecDeque::new(),
            dropouts: Vec::new(),
            peaks_from: if op.is_none() { Some(0) } else { None },
            op,
            written: 0,
            lookback: VecDeque::new(),
            lookback_cap: rate_hz as usize * LOOKBACK_S,
        }
    }

    /// Appends `part` (document-rate samples) to the take and its H-07 live peaks; a failure
    /// records [`Self::write_error`] and flags the control thread to stop the recording (H-05:
    /// the take is kept up to the last good sample, SPEC-002 §2.5). A no-op once `write_error` is
    /// already set, so callers (including the H-10 gap-splicing loop) can keep calling it after a
    /// failure without piling on more (and overwriting) errors.
    fn append_take(&mut self, part: &[f32]) {
        if part.is_empty() || self.write_error.is_some() {
            return;
        }
        match self.capture.append(part) {
            Ok(()) => self.feed_peaks(part),
            Err(e) => {
                self.write_error = Some(e);
                self.shared.writer_failed.store(true, Ordering::Release);
            }
        }
    }

    /// T-304 (SPEC-022 §4.5): live peaks describe the record window only. Until its start is
    /// known, appended samples wait in the look-back; then the part of it from `k_start` on is
    /// replayed and later samples go straight in.
    fn feed_peaks(&mut self, part: &[f32]) {
        let start = self.written;
        self.written += part.len() as u64;
        if self.peaks_from.is_none() {
            let k = self
                .op
                .as_ref()
                .map_or(0, |o| o.k_start.load(Ordering::Acquire));
            if k != u64::MAX {
                self.peaks_from = Some(k);
                let lb_start = start - self.lookback.len() as u64;
                let skip = k.saturating_sub(lb_start) as usize;
                if skip < self.lookback.len() {
                    let replay: Vec<f32> = self.lookback.iter().skip(skip).copied().collect();
                    self.peaks
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .append(&replay);
                }
                self.lookback = VecDeque::new();
            }
        }
        match self.peaks_from {
            Some(k) => {
                let skip = k.saturating_sub(start).min(part.len() as u64) as usize;
                if skip < part.len() {
                    self.peaks
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .append(&part[skip..]);
                }
            }
            None => {
                self.lookback.extend(part.iter().copied());
                let excess = self.lookback.len().saturating_sub(self.lookback_cap);
                self.lookback.drain(..excess);
            }
        }
    }

    /// Feeds `frames` device-rate samples of digital silence through the take path, in bounded
    /// chunks (H-10 item 4: a dropout fill). Goes through [`Self::drain_part`] rather than
    /// appending straight to [`Self::capture`], so a resampled take (H-06/H-11 item 5) converts
    /// the fill's duration to document-rate samples the same way it would real captured silence,
    /// instead of splicing in a mismatched number of raw zeros. No allocation beyond the
    /// resampler's own reused scratch buffer — [`SILENCE_CHUNK`] zeros are stack-local.
    fn fill_silence(&mut self, mut frames: u32) {
        let zeros = [0.0f32; SILENCE_CHUNK];
        while frames > 0 && self.write_error.is_none() {
            let n = (frames as usize).min(SILENCE_CHUNK);
            self.drain_part(&zeros[..n]);
            frames -= n as u32;
        }
    }

    /// Runs one drained ring segment (device-rate samples) through the resampler when one is
    /// configured, then appends whatever it produced; without one, appends `part` directly.
    fn drain_part(&mut self, part: &[f32]) {
        if part.is_empty() {
            return;
        }
        if self.resampler.is_some() {
            let mut scratch = std::mem::take(&mut self.resample_scratch);
            scratch.clear();
            if let Some(r) = self.resampler.as_mut() {
                r.push(part, &mut scratch);
            }
            if !scratch.is_empty() {
                self.append_take(&scratch);
            }
            scratch.clear();
            self.resample_scratch = scratch;
        } else {
            self.append_take(part);
        }
    }

    /// Like [`Self::drain_part`], but first splices in silence for every gap event whose position
    /// falls within `part` (H-10 item 4, H-11 item 5, SPEC-002 §2.4/§4.3) — `part` covers ring
    /// (device-rate) positions `[self.ring_consumed, self.ring_consumed + part.len())`, the same
    /// numbering [`GapEvent::take_index`] uses whether or not [`Self::resampler`] is active: the
    /// splice always happens on the raw, pre-resample stream, and the segments either side of it
    /// (pass-through audio and the fill) both go through [`Self::drain_part`], so a resampled take
    /// still ends up with the right *document*-rate marker position/length
    /// ([`Self::capture`]'s own sample count, read right before the fill).
    fn splice_gaps_and_append(&mut self, part: &[f32]) {
        if let Some(rx) = self.gap_rx.as_mut() {
            while let Ok(ev) = rx.pop() {
                self.pending_gaps.push_back(ev);
            }
        }
        let base = self.ring_consumed;
        let end = base + part.len() as u64;
        let mut cursor = 0usize;
        while let Some(&ev) = self.pending_gaps.front() {
            if ev.take_index >= end {
                break; // not reached by this segment yet
            }
            let local = ev.take_index.saturating_sub(base).min(part.len() as u64) as usize;
            if local > cursor {
                self.drain_part(&part[cursor..local]);
                cursor = local;
            }
            let pos_samples = self.capture.samples_written();
            // H-23 (SPEC-002 §4.3): an unreliable-timestamp gap's length is unknown — nothing is
            // filled, and the marker carries `DropoutMark::UNKNOWN_LEN` instead of a duration.
            let len_samples = if ev.lost_frames == UNKNOWN_GAP {
                DropoutMark::UNKNOWN_LEN
            } else {
                self.fill_silence(ev.lost_frames);
                u64::from(ev.lost_frames)
            };
            self.dropouts.push(DropoutMark {
                pos_samples,
                len_samples,
            });
            self.pending_gaps.pop_front();
        }
        if cursor < part.len() {
            self.drain_part(&part[cursor..]);
        }
    }

    /// Appends everything in the ring to the take (and its H-07 live peaks). Returns `true` once
    /// the take is complete and the ring is empty — and, for a record operation, once the control
    /// thread sealed its decision (T-304). After an append error the rest of the take is drained
    /// and dropped, and [`InputShared::writer_failed`] asks the control thread to stop the
    /// recording (H-05: the take is kept up to the last good sample, SPEC-002 §2.5).
    pub(crate) fn drain(&mut self) -> bool {
        // Read the completion flags first: every sample pushed before they were set is then
        // visible to the drain below (Release/Acquire).
        let complete = self.shared.capture_done.load(Ordering::Acquire)
            || self.shared.force_finish.load(Ordering::Acquire)
            || self.rx.is_abandoned();
        let sealed = self.op.as_ref().is_none_or(|o| o.is_sealed());
        let n = self.rx.slots();
        if n > 0
            && let Ok(chunk) = self.rx.read_chunk(n)
        {
            // Copy out of the ring and commit it before touching `self` as a whole below: `chunk`
            // borrows `self.rx`, so it must be gone before `drain_part` (a `&mut self` method,
            // needed for the resampler path) can run.
            let mut drained = std::mem::take(&mut self.drain_scratch);
            drained.clear();
            let (a, b) = chunk.as_slices();
            drained.extend_from_slice(a);
            drained.extend_from_slice(b);
            chunk.commit_all();
            if self.write_error.is_none() {
                if self.gap_rx.is_some() {
                    self.splice_gaps_and_append(&drained);
                } else {
                    self.drain_part(&drained);
                }
            }
            self.ring_consumed = self.ring_consumed.saturating_add(drained.len() as u64);
            drained.clear();
            self.drain_scratch = drained;
        }
        complete && sealed && self.rx.is_empty()
    }

    /// Patches the WAV header + `fdatasync`s on this thread (inline mode).
    pub(crate) fn sync(&mut self) {
        let _ = self.capture.sync();
    }

    /// Finishes the take, returns the ring consumer home, then calls the done callback. Flushes
    /// the resampler's tail first (H-06: the last, possibly partial, chunk of device-rate samples
    /// still buffered) — unless a write error already stopped appends for good.
    pub(crate) fn finish(mut self) {
        if self.write_error.is_none()
            && let Some(r) = self.resampler.take()
        {
            let mut scratch = std::mem::take(&mut self.resample_scratch);
            scratch.clear();
            r.finish(&mut scratch);
            if !scratch.is_empty() {
                self.append_take(&scratch);
            }
        }
        let reason = match self.shared.stop_reason.load(Ordering::Relaxed) {
            stop_code::INPUT_LOST => StopReason::InputLost,
            stop_code::SHUTDOWN => StopReason::Shutdown,
            stop_code::OVERFLOW => StopReason::Overflow,
            stop_code::WRITE_ERROR => StopReason::WriteError,
            stop_code::DISK_FULL => StopReason::DiskFull,
            _ => StopReason::User,
        };
        let finished = self.capture.finish();
        *self.home.lock().unwrap_or_else(PoisonError::into_inner) = Some(self.rx);
        if let Some(gap_rx) = self.gap_rx.take() {
            *self.gap_home.lock().unwrap_or_else(PoisonError::into_inner) = Some(gap_rx);
        }
        let result = RecordingResult {
            finished,
            reason,
            sample_rate_hz: self.rate_hz,
            clip_events: self.shared.clip_events.load(Ordering::Relaxed),
            overflow_samples: self.shared.overflow_samples.load(Ordering::Relaxed),
            write_error: self.write_error.take(),
            dropouts: std::mem::take(&mut self.dropouts),
            op: self.op.as_ref().and_then(|o| o.take_outcome()),
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
        let (len, spb, got) = peaks.snapshot(0, u32::MAX);
        assert_eq!(len, samples.len() as u64);
        assert_eq!(spb, LIVE_PEAKS_SPB, "far below the H-10 decimation cap");
        let want = brute_force(&samples, LIVE_PEAKS_SPB as usize);
        assert_eq!(got, want);
    }

    /// `snapshot` pages by bucket index and clamps a `start_bucket`/`max` past the end.
    #[test]
    fn snapshot_pages_and_clamps_start_bucket() {
        let samples: Vec<f32> = (0..u64::from(LIVE_PEAKS_SPB * 5) + 10).map(synth).collect();
        let mut peaks = LivePeaks::default();
        peaks.append(&samples);
        let (_, _, all) = peaks.snapshot(0, 100);
        assert_eq!(all.len(), 6, "5 full buckets + 1 partial");
        let (_, _, page) = peaks.snapshot(2, 2);
        assert_eq!(page, all[2..4]);
        let (_, _, past_end) = peaks.snapshot(50, 10);
        assert!(past_end.is_empty());
    }

    /// An empty take reports zero length and no buckets (no divide-by-zero / underflow on the
    /// first `drain()` before anything was appended).
    #[test]
    fn empty_live_peaks_snapshot_is_empty() {
        let peaks = LivePeaks::default();
        let (len, spb, buckets) = peaks.snapshot(0, 10);
        assert_eq!((len, spb, buckets.len()), (0, LIVE_PEAKS_SPB, 0));
    }

    /// H-10 item 6: once completed buckets reach the cap, they decimate (halve, `spb` doubles)
    /// instead of growing forever — memory stays bounded for a multi-hour take. The decimated
    /// peaks still cover every sample (brute-force min/max at the new resolution), and the take
    /// length is unaffected.
    #[test]
    fn decimates_once_the_bucket_cap_is_reached() {
        // A handful of buckets past the cap, forcing exactly one decimation pass.
        let extra_buckets = 5u64;
        let samples: Vec<f32> = (0..(u64::from(LIVE_PEAKS_SPB)
            * (MAX_LIVE_PEAK_BUCKETS as u64 + extra_buckets)))
            .map(synth)
            .collect();
        let mut peaks = LivePeaks::default();
        for chunk in samples.chunks(4001) {
            peaks.append(chunk);
        }
        assert_eq!(peaks.len_samples, samples.len() as u64, "no samples lost");
        let (len, spb, got) = peaks.snapshot(0, u32::MAX);
        assert_eq!(len, samples.len() as u64);
        assert_eq!(spb, LIVE_PEAKS_SPB * 2, "one decimation pass doubled it");
        assert!(
            got.len() < MAX_LIVE_PEAK_BUCKETS,
            "decimation must actually shrink the array: got {} buckets",
            got.len()
        );
        let want = brute_force(&samples, spb as usize);
        assert_eq!(got, want);
    }
}

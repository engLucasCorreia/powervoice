//! The input callback (ADR-002 §1–§2, SPEC-002 §2.1, §4.1, §4.2, §4.5, §4.6).
//!
//! Per device buffer — after T-102's [`MonoInput`] has deinterleaved the selected channel — the
//! callback drains the control commands, meters the block (peak, energy, clip), pushes it to the
//! monitor ring while monitoring, and, while a take is being captured, pushes the samples whose
//! capture time lies in `[t_record, t_stop)` to the capture ring. Alignment is computed from the
//! block's capture timestamp mapped to the app clock (`to_app_ns`, which uses the callback's one
//! clock read), so the take is exact to ±1 sample (ADR-002 §8). One [`InputEvent::Block`] per call
//! reports the meter and the take position to the control thread.
//!
//! RT contract: no allocation, locks, I/O or panics; the FTZ/DAZ guard is entered first. When the
//! stream ends, the backend drops the callback (cpal: on its own thread) and `Drop` hands the
//! monitor-ring producer back through a slot so the control thread can link it to the next input
//! stream. The capture-ring producer is simply dropped: the capture-writer sees the ring
//! abandoned and finishes the take with what it holds.

use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use rtrb::{Consumer, Producer};
use vox_dsp::fp::DenormalGuard;

use crate::backend::{InputTimestamp, frames_to_ns};
use crate::devices::{InputChannel, MonoInput};
use crate::monitor::MonitorLink;

/// Capture ring length (ADR-002 §2).
pub(crate) const CAPTURE_RING_SECONDS: usize = 10;
/// Control → input command ring capacity.
pub(crate) const INPUT_CMD_CAPACITY: usize = 64;
/// Input → control event ring capacity (ADR-002 §2).
pub(crate) const INPUT_EVENT_CAPACITY: usize = 1024;
/// Commands drained per call at most.
const MAX_INPUT_CMDS_PER_CALLBACK: usize = 16;
/// Frames deinterleaved per call (larger device buffers are split by `MonoInput`).
const MONO_SCRATCH_FRAMES: usize = 8192;
/// Clip threshold (SPEC-002 §3 `clip_threshold`).
pub(crate) const CLIP_THRESHOLD: f32 = 0.999_90;
/// Clip runs closer than this count as one event (SPEC-002 §3 `clip_merge_ms`).
const CLIP_MERGE_MS: u64 = 10;
/// H-10 item 4: the gap ring's capacity (SPEC-002 §4.3: "a small gap ring") — dropouts are rare,
/// so a handful of pending events is plenty; a full ring drops the newest event (RT-safe, same
/// convention as [`InputSide::emit`]).
pub(crate) const GAP_EVENT_CAPACITY: usize = 64;

/// Control → input callback.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum InputCmd {
    /// Start a take with the first sample captured at or after app time `start_ns` (0: the next
    /// sample). Resets the per-take counters.
    StartCapture { start_ns: u64 },
    /// End the take before app time `stop_ns` (exclusive).
    StopCapture { stop_ns: u64 },
    /// Push to the monitor ring (monitoring audible).
    Monitor(bool),
}

/// Input callback → control.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum InputEvent {
    /// One per call: meter values of the block; `captured` = take samples so far, reached at app
    /// time `end_ns` (the end of the block); `latency_ns` = the callback's app time − `end_ns`
    /// (the device's input latency, T-107's monitoring readout).
    Block {
        frames: u32,
        latency_ns: u32,
        peak: f32,
        sum_sq: f64,
        clipped: bool,
        capturing: bool,
        captured: u64,
        end_ns: u64,
    },
    /// The take's last sample is in the capture ring.
    CaptureEnded { samples: u64 },
}

/// H-10 item 4 (SPEC-002 §2.4/§4.3): one detected input dropout, in the ring-position numbering
/// the capture-writer also sees draining `InputSideParts::capture` (device-rate sample count
/// pushed to the capture ring since the take started). `take_index`: where the gap starts.
/// `lost_frames`: the estimated number of missing device-rate frames to fill with silence.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct GapEvent {
    pub(crate) take_index: u64,
    pub(crate) lost_frames: u32,
}

/// [`InputShared::stop_reason`] values.
pub(crate) mod stop_code {
    pub(crate) const USER: u8 = 0;
    pub(crate) const INPUT_LOST: u8 = 1;
    pub(crate) const SHUTDOWN: u8 = 2;
    /// H-05: the capture ring overflowed; the callback ended the take (SPEC-002 §2.4).
    pub(crate) const OVERFLOW: u8 = 3;
    /// H-05: appending to the take failed; the control thread stopped it (SPEC-002 §2.5).
    pub(crate) const WRITE_ERROR: u8 = 4;
    /// H-11: free space on the session volume fell below the hard floor; the control thread
    /// stopped it (SPEC-002 §2.5, AC-13).
    pub(crate) const DISK_FULL: u8 = 5;
}

/// Shared by the input callback, the control thread and the capture-writer (atomics only).
#[derive(Debug, Default)]
pub(crate) struct InputShared {
    /// Set by the callback once the take's last sample is in the capture ring (Release after
    /// the push, so a writer that sees it (Acquire) drains the whole take).
    pub(crate) capture_done: AtomicBool,
    /// Set by the control thread: finish with what the ring holds (input lost, stall, shutdown).
    pub(crate) force_finish: AtomicBool,
    /// Why the take ends ([`stop_code`]).
    pub(crate) stop_reason: AtomicU8,
    /// Clip events of the current take.
    pub(crate) clip_events: AtomicU32,
    /// H-10 item 4: dropout events detected so far this take (SPEC-002 §2.1's live amber
    /// counter) — incremented only for events the gap ring actually accepted, so this always
    /// matches the number of dropout markers the capture-writer will add.
    pub(crate) dropout_events: AtomicU32,
    /// Samples that didn't fit in the full capture ring: the take ended just before them.
    pub(crate) overflow_samples: AtomicU64,
    /// Events dropped because the event ring was full.
    pub(crate) dropped_events: AtomicU32,
    /// H-05: set by the capture-writer when appending to the take failed; the control thread
    /// then stops the take (the writer drops the rest of it).
    pub(crate) writer_failed: AtomicBool,
    /// H-11: set by the control thread's periodic disk-space check (SPEC-002 §2.5); mirrors
    /// `writer_failed` so `service_recording` stops the take the same way.
    pub(crate) disk_full: AtomicBool,
}

impl InputShared {
    /// \[control thread\] Resets the per-take state before `StartCapture` is sent.
    pub(crate) fn reset_take(&self) {
        self.capture_done.store(false, Ordering::Release);
        self.force_finish.store(false, Ordering::Release);
        self.writer_failed.store(false, Ordering::Release);
        self.disk_full.store(false, Ordering::Release);
        self.stop_reason.store(stop_code::USER, Ordering::Relaxed);
        self.clip_events.store(0, Ordering::Relaxed);
        self.dropout_events.store(0, Ordering::Relaxed);
        self.overflow_samples.store(0, Ordering::Relaxed);
    }
}

/// Monitor-ring producer tagged with the generation of the output stream whose ring it feeds,
/// with that ring's stamp (T-107: the output's phase-compensated fill) and the frames pushed so
/// far (moves with the producer when another input stream takes it over).
pub(crate) struct MonitorTx {
    pub(crate) generation: u32,
    pub(crate) producer: Producer<f32>,
    pub(crate) link: Arc<MonitorLink>,
    pub(crate) pushed: u64,
}
/// Where a dropped input callback leaves its monitor producer.
pub(crate) type MonitorSlot = Arc<Mutex<Option<MonitorTx>>>;

/// \[control thread\] Takes the producer a dropped callback left in `slot`.
pub(crate) fn take_monitor(slot: &MonitorSlot) -> Option<MonitorTx> {
    slot.lock().unwrap_or_else(PoisonError::into_inner).take()
}

/// Everything an input callback is built from.
pub(crate) struct InputSideParts {
    pub(crate) cmds: Consumer<InputCmd>,
    pub(crate) events: Producer<InputEvent>,
    pub(crate) capture: Producer<f32>,
    pub(crate) monitor: Option<MonitorTx>,
    pub(crate) shared: Arc<InputShared>,
    pub(crate) slot: MonitorSlot,
    pub(crate) rate_hz: u32,
    /// H-10 item 4: where detected dropouts are reported (SPEC-002 §4.3).
    pub(crate) gap_events: Producer<GapEvent>,
}

/// The mono part of the input callback (after deinterleaving).
pub(crate) struct InputSide {
    cmds: Consumer<InputCmd>,
    events: Producer<InputEvent>,
    capture: Producer<f32>,
    monitor: Option<MonitorTx>,
    monitor_on: bool,
    shared: Arc<InputShared>,
    slot: MonitorSlot,
    rate_hz: u32,
    clip_merge_samples: u64,
    capturing: bool,
    start_ns: u64,
    stop_ns: Option<u64>,
    captured: u64,
    last_clip: Option<u64>,
    /// H-10 item 4 (SPEC-002 §4.3): where dropouts are reported.
    gap_events: Producer<GapEvent>,
    /// The *stream-clock* capture time at the end of the last block processed while capturing
    /// (`None`: no reference yet — reset at `StartCapture`, so a gap is never reported across the
    /// arm/record boundary).
    last_capture_end_ns: Option<u64>,
    /// Frame count of the last block processed while capturing — the period estimate for the
    /// `0.5 × period` dropout threshold (SPEC-002 §4.3); adapts to the device's actual buffer
    /// size instead of assuming a fixed nominal one.
    last_block_frames: u32,
}

/// Index of the first sample at or after time offset `dt_ns` in a stream at `rate_hz`.
#[inline]
fn first_at_or_after(dt_ns: u64, rate_hz: u32) -> u64 {
    (u128::from(dt_ns) * u128::from(rate_hz)).div_ceil(1_000_000_000) as u64
}

impl InputSide {
    pub(crate) fn new(p: InputSideParts) -> Self {
        let rate_hz = p.rate_hz.max(1);
        Self {
            cmds: p.cmds,
            events: p.events,
            capture: p.capture,
            monitor: p.monitor,
            monitor_on: false,
            shared: p.shared,
            slot: p.slot,
            rate_hz,
            clip_merge_samples: u64::from(rate_hz) * CLIP_MERGE_MS / 1000,
            capturing: false,
            start_ns: 0,
            stop_ns: None,
            captured: 0,
            last_clip: None,
            gap_events: p.gap_events,
            last_capture_end_ns: None,
            last_block_frames: 0,
        }
    }

    fn emit(&mut self, e: InputEvent) {
        if self.events.push(e).is_err() {
            self.shared.dropped_events.fetch_add(1, Ordering::Relaxed);
        }
    }

    fn command(&mut self, cmd: InputCmd) {
        match cmd {
            InputCmd::StartCapture { start_ns } => {
                self.capturing = true;
                self.start_ns = start_ns;
                self.stop_ns = None;
                self.captured = 0;
                self.last_clip = None;
                self.last_capture_end_ns = None;
                self.last_block_frames = 0;
            }
            InputCmd::StopCapture { stop_ns } => {
                if self.capturing {
                    self.stop_ns = Some(stop_ns);
                }
            }
            InputCmd::Monitor(on) => self.monitor_on = on,
        }
    }

    /// Counts clip events in `seg` (take samples from index `self.captured`).
    fn count_clips(&mut self, seg: &[f32]) {
        for (i, &s) in seg.iter().enumerate() {
            if s.abs() >= CLIP_THRESHOLD {
                let idx = self.captured + i as u64;
                if self
                    .last_clip
                    .is_none_or(|last| idx - last > self.clip_merge_samples)
                {
                    self.shared.clip_events.fetch_add(1, Ordering::Relaxed);
                }
                self.last_clip = Some(idx);
            }
        }
    }

    /// H-10 item 4 (SPEC-002 §4.3): compares this block's *stream-clock* capture time (not the
    /// app-time-mapped `t0` — the formula is literally `capture_k`/`capture_{k+1}`, the device's
    /// own capture timestamps, since a device/driver-side dropout can leave the app-clock mapping
    /// undisturbed while the device's own reported position jumps) to the expected continuation
    /// of the previous block, and reports a gap event when it slipped by at least
    /// `max(0.5 × period, 1 ms)`. Called only while `self.capturing`, before this block's own
    /// samples are pushed (so `self.captured` is still the position right before the gap).
    ///
    /// H-11 item 4 (SPEC-002 §2.4 second row, A-011): a gap of more than
    /// [`crate::record::DROPOUT_FILL_MAX_S`] is device loss, not a fillable dropout — the take
    /// ends right here (like a capture-ring overflow, no gap event, no live counter bump) instead
    /// of being asked to silently fill seconds of silence.
    ///
    /// RT-safe: one non-blocking ring push, no allocation.
    fn detect_dropout(&mut self, capture_ns: u64) {
        if let Some(expected) = self.last_capture_end_ns
            && capture_ns > expected
        {
            let gap_ns = capture_ns - expected;
            let period_ns = frames_to_ns(u64::from(self.last_block_frames.max(1)), self.rate_hz);
            let min_gap_ns = (period_ns / 2).max(1_000_000);
            if gap_ns >= min_gap_ns {
                // Round to the nearest frame: (gap_ns * rate + 0.5 s worth of ns) / 1 s of ns.
                let lost =
                    (u128::from(gap_ns) * u128::from(self.rate_hz) + 500_000_000) / 1_000_000_000;
                let max_fill =
                    u128::from(self.rate_hz) * u128::from(crate::record::DROPOUT_FILL_MAX_S);
                if lost > max_fill {
                    // A-011: handled like device loss (SPEC-001 §2.4) — finalize at the last good
                    // sample and keep the take, no silence fill.
                    self.shared
                        .stop_reason
                        .store(stop_code::INPUT_LOST, Ordering::Relaxed);
                    self.end_capture();
                    return;
                }
                let lost_frames = u32::try_from(lost).unwrap_or(u32::MAX);
                if lost_frames > 0
                    && self
                        .gap_events
                        .push(GapEvent {
                            take_index: self.captured,
                            lost_frames,
                        })
                        .is_ok()
                {
                    self.shared.dropout_events.fetch_add(1, Ordering::Relaxed);
                }
            }
        }
    }

    /// Pushes the block's samples inside `[start, stop)` to the capture ring; `t0` = app time of
    /// `x[0]`.
    fn capture_block(&mut self, x: &[f32], t0: u64) {
        let n = x.len() as u64;
        let a = if self.start_ns > t0 {
            first_at_or_after(self.start_ns - t0, self.rate_hz).min(n)
        } else {
            0
        };
        let b = match self.stop_ns {
            Some(stop) if stop > t0 => first_at_or_after(stop - t0, self.rate_hz).min(n),
            Some(_) => 0,
            None => n,
        };
        if b > a {
            // Started: from now on the take continues with every sample.
            self.start_ns = 0;
            let seg = &x[a as usize..b as usize];
            let (pushed, rest) = self.capture.push_partial_slice(seg);
            self.count_clips(pushed);
            self.captured += pushed.len() as u64;
            if !rest.is_empty() {
                // H-05, SPEC-002 §2.4: the writer fell a whole ring (10 s) behind. Never splice
                // around the lost samples: the take ends at the last sample that fit.
                self.shared
                    .overflow_samples
                    .fetch_add(rest.len() as u64, Ordering::Relaxed);
                self.shared
                    .stop_reason
                    .store(stop_code::OVERFLOW, Ordering::Relaxed);
                self.end_capture();
                return;
            }
        }
        if let Some(stop) = self.stop_ns
            && t0.saturating_add(frames_to_ns(n, self.rate_hz)) >= stop
        {
            self.end_capture();
        }
    }

    /// The take's last sample is in the capture ring: tell the writer (Release after the push)
    /// and the control thread.
    fn end_capture(&mut self) {
        self.capturing = false;
        self.stop_ns = None;
        self.shared.capture_done.store(true, Ordering::Release);
        let samples = self.captured;
        self.emit(InputEvent::CaptureEnded { samples });
    }

    /// One mono block captured from app time `ts.to_app_ns(ts.capture_ns)`. RT-safe.
    pub(crate) fn process(&mut self, x: &[f32], ts: InputTimestamp) {
        let _fp = DenormalGuard::new();
        for _ in 0..MAX_INPUT_CMDS_PER_CALLBACK {
            match self.cmds.pop() {
                Ok(cmd) => self.command(cmd),
                Err(_) => break,
            }
        }
        if x.is_empty() {
            return;
        }
        let t0 = ts.to_app_ns(ts.capture_ns);
        let frames = x.len() as u64;
        if self.capturing {
            // H-10 item 4 (SPEC-002 §4.3): detect a gap before this block's own samples advance
            // `self.captured` — the reported `take_index` must be the position right before it.
            // Uses the stream's own (unmapped) capture time — see `detect_dropout`'s docs.
            self.detect_dropout(ts.capture_ns);
        }
        let mut peak = 0.0f32;
        let mut sum_sq = 0.0f64;
        for &s in x {
            peak = peak.max(s.abs());
            sum_sq += f64::from(s) * f64::from(s);
        }
        if self.monitor_on
            && let Some(m) = self.monitor.as_mut()
        {
            // A full monitor ring drops the rest (the output side drops back to F*). The stamp
            // follows the push: the output measures the fill against this callback's time.
            let (pushed, _) = m.producer.push_partial_slice(x);
            m.pushed = m.pushed.wrapping_add(pushed.len() as u64);
            m.link.publish(m.pushed, ts.now_ns);
        }
        if self.capturing {
            self.capture_block(x, t0);
        }
        // H-10 item 4: the reference for the *next* block's gap check — `None` while not
        // capturing (including just-ended-this-call, e.g. overflow/stop) so a future take starts
        // fresh (`StartCapture` also resets this, belt and suspenders).
        if self.capturing {
            self.last_capture_end_ns = Some(
                ts.capture_ns
                    .saturating_add(frames_to_ns(frames, self.rate_hz)),
            );
            self.last_block_frames = frames as u32;
        } else {
            self.last_capture_end_ns = None;
        }
        let end_ns = t0.saturating_add(frames_to_ns(frames, self.rate_hz));
        self.emit(InputEvent::Block {
            frames: frames as u32,
            latency_ns: u32::try_from(ts.now_ns.saturating_sub(end_ns)).unwrap_or(u32::MAX),
            peak,
            sum_sq,
            clipped: peak >= CLIP_THRESHOLD,
            capturing: self.capturing,
            captured: self.captured,
            end_ns,
        });
    }
}

impl Drop for InputSide {
    fn drop(&mut self) {
        if let Some(m) = self.monitor.take() {
            *self.slot.lock().unwrap_or_else(PoisonError::into_inner) = Some(m);
        }
    }
}

/// The input stream's callback: deinterleave the selected channel (`MonoInput`), then `side`.
pub(crate) fn input_callback(
    channel: InputChannel,
    mut side: InputSide,
) -> MonoInput<impl FnMut(&[f32], InputTimestamp) + Send + 'static> {
    let rate = side.rate_hz;
    MonoInput::new(channel, rate, MONO_SCRATCH_FRAMES, move |x, ts| {
        side.process(x, ts)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rtrb::RingBuffer;
    use vox_module_api::test_util::no_alloc;

    fn side(
        rate: u32,
    ) -> (
        InputSide,
        Producer<InputCmd>,
        Consumer<f32>,
        Arc<InputShared>,
        Consumer<GapEvent>,
    ) {
        let (cmd_tx, cmd_rx) = RingBuffer::new(INPUT_CMD_CAPACITY);
        let (ev_tx, ev_rx) = RingBuffer::new(INPUT_EVENT_CAPACITY);
        drop(ev_rx); // pushes into an abandoned ring still succeed
        let (cap_tx, cap_rx) = RingBuffer::new(4096);
        let (gap_tx, gap_rx) = RingBuffer::new(GAP_EVENT_CAPACITY);
        let shared = Arc::new(InputShared::default());
        let s = InputSide::new(InputSideParts {
            cmds: cmd_rx,
            events: ev_tx,
            capture: cap_tx,
            monitor: None,
            shared: shared.clone(),
            slot: Arc::new(Mutex::new(None)),
            rate_hz: rate,
            gap_events: gap_tx,
        });
        (s, cmd_tx, cap_rx, shared, gap_rx)
    }

    fn ts(t_ns: u64) -> InputTimestamp {
        InputTimestamp {
            now_ns: t_ns + 1_000_000,
            callback_ns: 5_000_000 + t_ns + 1_000_000,
            capture_ns: 5_000_000 + t_ns,
        }
    }

    /// Start/stop trimming by capture time: 1 kHz stream, 1 ms per sample.
    #[test]
    fn capture_is_trimmed_to_start_and_stop_times() {
        let (mut s, mut cmds, mut cap, shared, _gaps) = side(1000);
        let x: Vec<f32> = (0..10).map(|i| i as f32).collect();
        cmds.push(InputCmd::StartCapture {
            start_ns: 3_500_000,
        })
        .unwrap();
        no_alloc(|| s.process(&x, ts(0))).unwrap();
        cmds.push(InputCmd::StopCapture {
            stop_ns: 16_000_000,
        })
        .unwrap();
        let y: Vec<f32> = (10..20).map(|i| i as f32).collect();
        no_alloc(|| s.process(&y, ts(10_000_000))).unwrap();
        let mut got = Vec::new();
        while let Ok(v) = cap.pop() {
            got.push(v);
        }
        let want: Vec<f32> = (4..16).map(|i| i as f32).collect();
        assert_eq!(got, want, "samples at t ∈ [3.5 ms, 16 ms)");
        assert!(shared.capture_done.load(Ordering::Acquire));
    }

    /// H-05 (SPEC-002 §2.4, AC-8): a full capture ring ends the take at the last sample that
    /// fit — it never drops samples from the middle and carries on (a silent splice). Later
    /// blocks push nothing; the reason and the lost count are reported; no allocation.
    #[test]
    fn capture_ring_overflow_ends_the_take_at_the_last_sample_that_fit() {
        let (mut s, mut cmds, mut cap, shared, _gaps) = side(1000);
        cmds.push(InputCmd::StartCapture { start_ns: 0 }).unwrap();
        // The ring holds 4096 samples; the writer is stalled (nothing is popped).
        let x: Vec<f32> = (0..3000).map(|i| i as f32).collect();
        no_alloc(|| s.process(&x, ts(0))).unwrap();
        assert!(!shared.capture_done.load(Ordering::Acquire), "still room");
        let y: Vec<f32> = (3000..6000).map(|i| i as f32).collect();
        no_alloc(|| s.process(&y, ts(3_000_000_000))).unwrap();
        assert!(shared.capture_done.load(Ordering::Acquire), "take ended");
        assert_eq!(
            shared.stop_reason.load(Ordering::Relaxed),
            stop_code::OVERFLOW
        );
        assert_eq!(shared.overflow_samples.load(Ordering::Relaxed), 6000 - 4096);
        // The writer catches up: later blocks are no longer part of the take.
        let mut got = Vec::new();
        while let Ok(v) = cap.pop() {
            got.push(v);
        }
        let z: Vec<f32> = (6000..7000).map(|i| i as f32).collect();
        no_alloc(|| s.process(&z, ts(6_000_000_000))).unwrap();
        assert!(cap.pop().is_err(), "nothing captured after the overflow");
        let want: Vec<f32> = (0..4096).map(|i| i as f32).collect();
        assert_eq!(got, want, "a contiguous prefix of the input, no splice");
    }

    /// Clip runs closer than 10 ms merge into one event.
    #[test]
    fn clip_runs_merge_within_10_ms() {
        let (mut s, mut cmds, _cap, shared, _gaps) = side(1000);
        cmds.push(InputCmd::StartCapture { start_ns: 0 }).unwrap();
        let mut x = vec![0.0f32; 100];
        x[5] = 1.0; // event 1
        x[12] = -1.0; // 7 ms later: same event
        x[40] = 0.99995; // event 2
        x[60] = 0.9998; // below the threshold
        no_alloc(|| s.process(&x, ts(0))).unwrap();
        assert_eq!(shared.clip_events.load(Ordering::Relaxed), 2);
    }

    /// H-10 item 4 (SPEC-002 §4.3, AC-7): a 10 ms gap between two 10 ms blocks (period-based
    /// threshold 5 ms) is reported at the take index right before it, with the lost frames
    /// rounded from the gap duration; the live counter reflects it. No allocation.
    #[test]
    fn dropout_gap_reports_take_index_and_lost_frames() {
        let (mut s, mut cmds, _cap, shared, mut gaps) = side(1000);
        cmds.push(InputCmd::StartCapture { start_ns: 0 }).unwrap();
        let block1: Vec<f32> = vec![0.0; 10];
        no_alloc(|| s.process(&block1, ts(0))).unwrap();
        assert!(
            gaps.pop().is_err(),
            "no gap on the first block (no reference yet)"
        );

        // Block 1 ends at 10 ms; this block starts at 20 ms — a 10 ms gap.
        let block2: Vec<f32> = vec![0.0; 10];
        no_alloc(|| s.process(&block2, ts(20_000_000))).unwrap();

        let ev = gaps.pop().expect("the gap must be reported");
        assert_eq!(
            ev,
            GapEvent {
                take_index: 10,
                lost_frames: 10
            }
        );
        assert_eq!(shared.dropout_events.load(Ordering::Relaxed), 1);
        assert!(gaps.pop().is_err(), "only one event for one gap");
    }

    /// A gap below `max(0.5 × period, 1 ms)` is not a dropout (SPEC-002 §4.3).
    #[test]
    fn dropout_below_threshold_is_not_reported() {
        let (mut s, mut cmds, _cap, shared, mut gaps) = side(1000);
        cmds.push(InputCmd::StartCapture { start_ns: 0 }).unwrap();
        let block1: Vec<f32> = vec![0.0; 10]; // 10 ms block ⇒ 5 ms threshold
        no_alloc(|| s.process(&block1, ts(0))).unwrap();
        // Block 1 ends at 10 ms; this one starts at 13 ms — only a 3 ms gap.
        let block2: Vec<f32> = vec![0.0; 10];
        no_alloc(|| s.process(&block2, ts(13_000_000))).unwrap();

        assert!(gaps.pop().is_err(), "below threshold: not a dropout");
        assert_eq!(shared.dropout_events.load(Ordering::Relaxed), 0);
    }

    /// H-11 item 4 (SPEC-002 §2.4 second row, A-011): a gap longer than the 2 s fill cap is
    /// handled like device loss — the take ends right at the gap (no gap event, no live dropout
    /// counter bump), not filled with 2+ seconds of silence.
    #[test]
    fn dropout_longer_than_the_fill_cap_ends_the_take_like_device_loss() {
        let (mut s, mut cmds, mut cap, shared, mut gaps) = side(1000);
        cmds.push(InputCmd::StartCapture { start_ns: 0 }).unwrap();
        let block1: Vec<f32> = vec![1.0; 10];
        no_alloc(|| s.process(&block1, ts(0))).unwrap();
        assert!(!shared.capture_done.load(Ordering::Acquire));

        // Block 1 ends at 10 ms; this one starts 2001 ms later — past the 2 s (2000 ms) cap.
        let block2: Vec<f32> = vec![2.0; 10];
        no_alloc(|| s.process(&block2, ts(2_011_000_000))).unwrap();

        assert!(
            gaps.pop().is_err(),
            "too long to be a fillable dropout: no gap event"
        );
        assert_eq!(
            shared.dropout_events.load(Ordering::Relaxed),
            0,
            "not counted as a (fillable) dropout"
        );
        assert!(shared.capture_done.load(Ordering::Acquire), "take ended");
        assert_eq!(
            shared.stop_reason.load(Ordering::Relaxed),
            stop_code::INPUT_LOST,
            "handled like device loss (SPEC-002 §2.4 second row)"
        );
        // Only block 1's samples are in the take: block 2 arrived after the gap ended it.
        let mut got = Vec::new();
        while let Ok(v) = cap.pop() {
            got.push(v);
        }
        assert_eq!(got, block1);
    }

    /// A gap of exactly the 2 s fill cap is still a fillable dropout — only gaps *longer* than it
    /// cut over to device loss (SPEC-002 §2.4: "at most 2 s" is filled).
    #[test]
    fn dropout_at_exactly_the_fill_cap_is_still_filled() {
        let (mut s, mut cmds, _cap, shared, mut gaps) = side(1000);
        cmds.push(InputCmd::StartCapture { start_ns: 0 }).unwrap();
        let block1: Vec<f32> = vec![0.0; 10];
        no_alloc(|| s.process(&block1, ts(0))).unwrap();
        // Block 1 ends at 10 ms; this one starts exactly 2 s later.
        let block2: Vec<f32> = vec![0.0; 10];
        no_alloc(|| s.process(&block2, ts(2_010_000_000))).unwrap();

        let ev = gaps.pop().expect("a 2 s gap is still filled, not cut over");
        assert_eq!(ev.lost_frames, 2000);
        assert_eq!(shared.dropout_events.load(Ordering::Relaxed), 1);
        assert!(!shared.capture_done.load(Ordering::Acquire), "still going");
    }

    /// `reset_take` (called before every `StartCapture`) clears the live dropout counter, and a
    /// fresh take starts with no gap reference from the previous one (no false positive right
    /// after Start even if the input was armed a while before it, SPEC-002 §2.1).
    #[test]
    fn dropout_counter_resets_and_no_reference_carries_across_takes() {
        let (mut s, mut cmds, _cap, shared, mut gaps) = side(1000);
        cmds.push(InputCmd::StartCapture { start_ns: 0 }).unwrap();
        let block: Vec<f32> = vec![0.0; 10];
        no_alloc(|| s.process(&block, ts(0))).unwrap();
        no_alloc(|| s.process(&block, ts(20_000_000))).unwrap(); // a real 10 ms dropout
        assert_eq!(shared.dropout_events.load(Ordering::Relaxed), 1);
        let _ = gaps.pop();

        cmds.push(InputCmd::StopCapture {
            stop_ns: 30_000_000,
        })
        .unwrap();
        no_alloc(|| s.process(&block, ts(30_000_000))).unwrap();
        shared.reset_take();
        assert_eq!(shared.dropout_events.load(Ordering::Relaxed), 0);

        // A new take, long after the old one's last block — no false dropout on its first block.
        cmds.push(InputCmd::StartCapture {
            start_ns: 5_000_000_000,
        })
        .unwrap();
        no_alloc(|| s.process(&block, ts(5_000_000_000))).unwrap();
        assert!(
            gaps.pop().is_err(),
            "no reference should carry across takes"
        );
        assert_eq!(shared.dropout_events.load(Ordering::Relaxed), 0);
    }
}

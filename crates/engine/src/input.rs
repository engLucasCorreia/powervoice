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

/// Capture ring length (ADR-002 §2).
pub(crate) const CAPTURE_RING_SECONDS: usize = 10;
/// Monitor ring capacity (ADR-002 §2).
pub(crate) const MONITOR_RING_FRAMES: usize = 8192;
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
    /// time `end_ns` (the end of the block).
    Block {
        frames: u32,
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

/// [`InputShared::stop_reason`] values.
pub(crate) mod stop_code {
    pub(crate) const USER: u8 = 0;
    pub(crate) const INPUT_LOST: u8 = 1;
    pub(crate) const SHUTDOWN: u8 = 2;
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
    /// Samples lost to a full capture ring in the current take.
    pub(crate) overflow_samples: AtomicU64,
    /// Events dropped because the event ring was full.
    pub(crate) dropped_events: AtomicU32,
}

impl InputShared {
    /// \[control thread\] Resets the per-take state before `StartCapture` is sent.
    pub(crate) fn reset_take(&self) {
        self.capture_done.store(false, Ordering::Release);
        self.force_finish.store(false, Ordering::Release);
        self.stop_reason.store(stop_code::USER, Ordering::Relaxed);
        self.clip_events.store(0, Ordering::Relaxed);
        self.overflow_samples.store(0, Ordering::Relaxed);
    }
}

/// Monitor-ring producer tagged with the generation of the output stream whose ring it feeds.
pub(crate) type MonitorTx = (u32, Producer<f32>);
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
            self.count_clips(seg);
            let (_, rest) = self.capture.push_partial_slice(seg);
            if !rest.is_empty() {
                self.shared
                    .overflow_samples
                    .fetch_add(rest.len() as u64, Ordering::Relaxed);
            }
            self.captured += seg.len() as u64;
        }
        if let Some(stop) = self.stop_ns
            && t0.saturating_add(frames_to_ns(n, self.rate_hz)) >= stop
        {
            self.capturing = false;
            self.stop_ns = None;
            self.shared.capture_done.store(true, Ordering::Release);
            let samples = self.captured;
            self.emit(InputEvent::CaptureEnded { samples });
        }
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
        let mut peak = 0.0f32;
        let mut sum_sq = 0.0f64;
        for &s in x {
            peak = peak.max(s.abs());
            sum_sq += f64::from(s) * f64::from(s);
        }
        if self.monitor_on
            && let Some((_, p)) = self.monitor.as_mut()
        {
            // A full monitor ring drops the rest (the output side re-primes).
            let _ = p.push_partial_slice(x);
        }
        if self.capturing {
            self.capture_block(x, t0);
        }
        let frames = x.len() as u64;
        self.emit(InputEvent::Block {
            frames: frames as u32,
            peak,
            sum_sq,
            clipped: peak >= CLIP_THRESHOLD,
            capturing: self.capturing,
            captured: self.captured,
            end_ns: t0.saturating_add(frames_to_ns(frames, self.rate_hz)),
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
    ) {
        let (cmd_tx, cmd_rx) = RingBuffer::new(INPUT_CMD_CAPACITY);
        let (ev_tx, ev_rx) = RingBuffer::new(INPUT_EVENT_CAPACITY);
        drop(ev_rx); // pushes into an abandoned ring still succeed
        let (cap_tx, cap_rx) = RingBuffer::new(4096);
        let shared = Arc::new(InputShared::default());
        let s = InputSide::new(InputSideParts {
            cmds: cmd_rx,
            events: ev_tx,
            capture: cap_tx,
            monitor: None,
            shared: shared.clone(),
            slot: Arc::new(Mutex::new(None)),
            rate_hz: rate,
        });
        (s, cmd_tx, cap_rx, shared)
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
        let (mut s, mut cmds, mut cap, shared) = side(1000);
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

    /// Clip runs closer than 10 ms merge into one event.
    #[test]
    fn clip_runs_merge_within_10_ms() {
        let (mut s, mut cmds, _cap, shared) = side(1000);
        cmds.push(InputCmd::StartCapture { start_ns: 0 }).unwrap();
        let mut x = vec![0.0f32; 100];
        x[5] = 1.0; // event 1
        x[12] = -1.0; // 7 ms later: same event
        x[40] = 0.99995; // event 2
        x[60] = 0.9998; // below the threshold
        no_alloc(|| s.process(&x, ts(0))).unwrap();
        assert_eq!(shared.clip_events.load(Ordering::Relaxed), 2);
    }
}

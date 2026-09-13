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
use crate::record::{RecordDone, RecordingResult, StopReason};

/// Drain period of the writer thread (≤ 50 ms, SPEC-002 §4.1).
pub(crate) const DRAIN_INTERVAL: Duration = Duration::from_millis(20);

/// Where the writer returns the capture-ring consumer when a take is finished.
pub(crate) type CaptureHome = Arc<Mutex<Option<Consumer<f32>>>>;

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
}

impl CaptureWriter {
    pub(crate) fn new(
        rx: Consumer<f32>,
        capture: TakeCapture,
        shared: Arc<InputShared>,
        home: CaptureHome,
        rate_hz: u32,
        done: RecordDone,
    ) -> Self {
        Self {
            rx,
            capture,
            shared,
            home,
            rate_hz,
            write_error: None,
            done: Some(done),
        }
    }

    /// Appends everything in the ring to the take. Returns `true` once the take is complete and
    /// the ring is empty. After an append error the rest of the take is drained and dropped
    /// (disk rules: hardening).
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
                    if let Err(e) = self.capture.append(part) {
                        self.write_error = Some(e);
                        break;
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

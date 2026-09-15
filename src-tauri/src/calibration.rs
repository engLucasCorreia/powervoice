//! Latency calibration job (T-304, SPEC-022 §2.14, §4.7): starts the engine's loopback run
//! (5 sweeps after the rack, monitoring forced off), reports `job_progress`
//! (`JobKind::Calibration`) at ~10 Hz, analyses the capture with `vox_dsp::calibration` on its
//! own worker thread and emits `calibration_result`. Applying the offset is a settings write
//! (`record_offset_set`); a rejected run never changes it.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tauri::{AppHandle, Emitter, Runtime};
use vox_engine::EngineHandle;
use vox_engine::record::{CalibrationError, CalibrationStatus};

use crate::ipc::{
    CalibrationResultDto, EventName, IpcError, IpcErrorCode, JobKind, JobProgressDto, JobState,
    Notice, NoticeLevel, emit_job_progress, emit_notice,
};

/// How often the worker polls the engine (and so the `job_progress` rate).
const POLL: Duration = Duration::from_millis(100);

/// What the service tells the UI.
pub enum CalibrationEvent {
    Progress(JobProgressDto),
    Result(CalibrationResultDto),
    /// H-30: a failed run's reason, posted before the terminal `Failed` progress event (every
    /// other job service's `fail_job` convention) — the calibration wizard's own "failed" stage
    /// (`record.svelte.ts::onJobProgress`) used to show no reason at all.
    Notice(Notice),
}

/// Emits [`CalibrationEvent`]s (built by [`start`] from the app handle).
pub type CalibrationEmitter = Arc<dyn Fn(CalibrationEvent) + Send + Sync>;

struct Inner {
    engine: EngineHandle,
    emit: CalibrationEmitter,
    next_job: AtomicU32,
    running: Mutex<Option<u32>>,
}

/// Cheaply cloneable, managed as Tauri state.
#[derive(Clone)]
pub struct CalibrationService(Arc<Inner>);

/// Builds the service for the running app.
pub fn start<R: Runtime>(app: &AppHandle<R>, engine: EngineHandle) -> CalibrationService {
    let app = app.clone();
    CalibrationService::new(
        engine,
        Arc::new(move |event| {
            let result = match event {
                CalibrationEvent::Progress(p) => emit_job_progress(&app, p),
                CalibrationEvent::Result(r) => app.emit(EventName::calibration_result.as_str(), r),
                CalibrationEvent::Notice(n) => emit_notice(&app, n),
            };
            if let Err(e) = result {
                tracing::warn!(error = %e, "emitting a calibration event failed");
            }
        }),
    )
}

/// Engine refusal → IPC error (SPEC-022 §2.14: refused while recording; needs both devices).
pub(crate) fn calibration_error(e: CalibrationError) -> IpcError {
    match e {
        CalibrationError::Recording => IpcError::not_while_recording(),
        CalibrationError::Busy => IpcError::new(IpcErrorCode::Busy, "error.calibration.busy"),
        CalibrationError::NoOutput => {
            IpcError::new(IpcErrorCode::DeviceNotFound, "error.calibration.no_output")
        }
        CalibrationError::NoInput => {
            IpcError::new(IpcErrorCode::DeviceNotFound, "error.calibration.no_input")
        }
        CalibrationError::RateMismatch {
            input_hz,
            output_hz,
        } => IpcError::new(
            IpcErrorCode::InvalidArgument,
            "error.calibration.rate_mismatch",
        )
        .with_param("input", input_hz.to_string())
        .with_param("output", output_hz.to_string()),
        CalibrationError::Interrupted => {
            IpcError::new(IpcErrorCode::DeviceLost, "error.calibration.interrupted")
        }
        CalibrationError::Cancelled => IpcError::new(IpcErrorCode::Cancelled, "error.cancelled"),
    }
}

/// H-30: mirrors every other job service's `notice_from_error` (export.rs, loudness.rs, ...).
fn notice_from_error(err: &IpcError) -> Notice {
    let mut notice = Notice::toast(NoticeLevel::Error, err.key.clone());
    for (name, value) in &err.params {
        notice = notice.with_param(name.clone(), value.clone());
    }
    notice
}

impl CalibrationService {
    pub fn new(engine: EngineHandle, emit: CalibrationEmitter) -> Self {
        Self(Arc::new(Inner {
            engine,
            emit,
            next_job: AtomicU32::new(0),
            running: Mutex::new(None),
        }))
    }

    /// Starts a run; `verify_offset_ms`: the offset to verify with (`None`: a measurement at
    /// δ = 0). Returns the job id its `job_progress` / `calibration_result` events carry.
    pub fn run(&self, verify_offset_ms: Option<f64>) -> Result<u32, IpcError> {
        let inner = &self.0;
        if inner.running.lock().unwrap().is_some() {
            return Err(calibration_error(CalibrationError::Busy));
        }
        let offset_ns = verify_offset_ms.map_or(0, vox_engine::record_op::offset_ms_to_ns);
        inner
            .engine
            .calibration_start(offset_ns)
            .map_err(calibration_error)?;
        let job_id = inner.next_job.fetch_add(1, Ordering::Relaxed) + 1;
        *inner.running.lock().unwrap() = Some(job_id);
        let worker = Arc::clone(inner);
        let verify = verify_offset_ms.is_some();
        std::thread::Builder::new()
            .name("vox-calibration".into())
            .spawn(move || worker.watch(job_id, verify))
            .map_err(|e| {
                inner.engine.calibration_cancel();
                *inner.running.lock().unwrap() = None;
                IpcError::internal(e.to_string())
            })?;
        Ok(job_id)
    }

    /// Aborts the running calibration (monitoring and arming are restored by the engine).
    pub fn cancel(&self) {
        self.0.engine.calibration_cancel();
    }
}

impl Inner {
    fn progress(&self, job_id: u32, state: JobState, fraction: f32) {
        (self.emit)(CalibrationEvent::Progress(JobProgressDto {
            job_id,
            kind: JobKind::Calibration,
            state,
            fraction,
        }));
    }

    /// H-30: reports a failed run — the error notice goes out before the terminal `Failed`
    /// progress event (every other job service's `fail_job` convention), so the UI's job store
    /// and tests see the reason as soon as they see `Failed`.
    fn fail(&self, job_id: u32, error: CalibrationError) {
        (self.emit)(CalibrationEvent::Notice(notice_from_error(
            &calibration_error(error),
        )));
        self.progress(job_id, JobState::Failed, 0.0);
    }

    /// The job's worker: polls the engine, analyses the finished capture, reports.
    fn watch(&self, job_id: u32, verify: bool) {
        self.progress(job_id, JobState::Running, 0.0);
        loop {
            std::thread::sleep(POLL);
            match self.engine.calibration_poll() {
                CalibrationStatus::Running { progress } => {
                    self.progress(job_id, JobState::Running, progress);
                }
                CalibrationStatus::Done(Ok(capture)) => {
                    let result = vox_dsp::calibration::analyze(
                        &capture.recording,
                        &capture.rep_starts,
                        &capture.sweep,
                        capture.rate_hz,
                    );
                    tracing::info!(
                        verify,
                        offset_ms = result.offset_ms,
                        confidence = result.confidence,
                        accepted = result.accepted,
                        "latency calibration finished"
                    );
                    (self.emit)(CalibrationEvent::Result(CalibrationResultDto::new(
                        job_id,
                        verify,
                        capture.rate_hz,
                        &result,
                    )));
                    self.progress(job_id, JobState::Done, 1.0);
                    break;
                }
                CalibrationStatus::Done(Err(CalibrationError::Cancelled)) => {
                    self.progress(job_id, JobState::Cancelled, 0.0);
                    break;
                }
                CalibrationStatus::Done(Err(error)) => {
                    tracing::warn!(%error, "latency calibration failed");
                    self.fail(job_id, error);
                    break;
                }
                CalibrationStatus::Idle => {
                    // Defensive: the run's own state vanished without a `Done` (should never
                    // happen — `run` just started it). Reported as `Interrupted` so the wizard's
                    // failed stage still gets a reason instead of none at all.
                    tracing::warn!("latency calibration polled Idle while a job was running");
                    self.fail(job_id, CalibrationError::Interrupted);
                    break;
                }
            }
        }
        *self.running.lock().unwrap() = None;
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex as StdMutex;

    use vox_engine::backend::fake::FakeBackend;
    use vox_engine::{Engine, EngineConfig};
    use vox_rack::Registry;

    use super::*;

    /// Refusals map to their i18n keys (SPEC-022 §2.14, AC-12).
    #[test]
    fn engine_refusals_map_to_i18n_keys() {
        assert_eq!(
            calibration_error(CalibrationError::Recording).key,
            "error.not_while_recording"
        );
        assert_eq!(
            calibration_error(CalibrationError::NoOutput).key,
            "error.calibration.no_output"
        );
        let e = calibration_error(CalibrationError::RateMismatch {
            input_hz: 44_100,
            output_hz: 48_000,
        });
        assert_eq!(e.params.get("input").map(String::as_str), Some("44100"));
    }

    #[derive(Debug, Clone, PartialEq)]
    enum TestEvent {
        Progress { state: JobState, fraction: f32 },
        Notice(String),
    }

    /// An `Inner::fail` needs an `EngineHandle` field to exist but never calls it — a stopped,
    /// device-free fake engine is enough (mirrors `nr_capture.rs`'s `stopped_engine`).
    fn stopped_engine() -> (Engine, EngineHandle) {
        let fake = FakeBackend::new(1);
        let registry =
            Arc::new(Registry::with_factories(vox_modules::builtin_factories()).unwrap());
        let engine = Engine::start(EngineConfig::new(Arc::new(fake), registry)).unwrap();
        let handle = engine.handle();
        (engine, handle)
    }

    /// H-30: a failed run posts the error notice before the terminal `Failed` progress event
    /// (every other job service's convention) — checked directly against `Inner::fail`, since
    /// `watch`'s own loop needs a real hardware-backed calibration run to reach it.
    #[test]
    fn a_failed_run_posts_the_notice_before_the_terminal_failed_progress_event() {
        let (_engine, handle) = stopped_engine();
        let events: Arc<StdMutex<Vec<TestEvent>>> = Arc::new(StdMutex::new(Vec::new()));
        let sink = Arc::clone(&events);
        let emit: CalibrationEmitter = Arc::new(move |event| {
            let e = match event {
                CalibrationEvent::Progress(dto) => TestEvent::Progress {
                    state: dto.state,
                    fraction: dto.fraction,
                },
                CalibrationEvent::Notice(n) => TestEvent::Notice(n.key),
                CalibrationEvent::Result(_) => return,
            };
            sink.lock().unwrap().push(e);
        });
        let service = CalibrationService::new(handle, emit);

        service.0.fail(
            7,
            CalibrationError::RateMismatch {
                input_hz: 44_100,
                output_hz: 48_000,
            },
        );

        let got = events.lock().unwrap().clone();
        assert_eq!(
            got,
            vec![
                TestEvent::Notice("error.calibration.rate_mismatch".to_string()),
                TestEvent::Progress {
                    state: JobState::Failed,
                    fraction: 0.0
                },
            ]
        );
    }
}

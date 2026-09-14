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
    emit_job_progress,
};

/// How often the worker polls the engine (and so the `job_progress` rate).
const POLL: Duration = Duration::from_millis(100);

/// What the service tells the UI.
pub enum CalibrationEvent {
    Progress(JobProgressDto),
    Result(CalibrationResultDto),
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
                    self.progress(job_id, JobState::Failed, 0.0);
                    break;
                }
                CalibrationStatus::Idle => {
                    self.progress(job_id, JobState::Failed, 0.0);
                    break;
                }
            }
        }
        *self.running.lock().unwrap() = None;
    }
}

#[cfg(test)]
mod tests {
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
}

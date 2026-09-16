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
                    // H-50 audit: already correctly ordered — `Result` goes out before the
                    // terminal `Done` progress event below, so anything that sees `Done` already
                    // has it (no reordering needed here, unlike normalize/export/loudness/bake).
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

    use vox_engine::backend::fake::{FakeBackend, FakeDriver, Loopback};
    use vox_engine::{DevicePrefs, Engine, EngineConfig};
    use vox_rack::Registry;

    use crate::settings::{RecordOffsetEntry, RecordOffsetSource, Settings};

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

    // --- H-53: the success path, end to end through a real engine -------------------------------

    /// A live engine (fake `"DAC"`/`"Mic"` loopback devices) for the success-path test below —
    /// mirrors `nr_capture.rs`'s `live_engine_with_nr_slot` / `export.rs`'s
    /// `live_engine_with_gain_minus_6_db`: a real threaded [`Engine`], kept alive with its own
    /// `FakeDriver` (real time), because `Inner::watch`'s worker thread polls the engine handle
    /// over real time too (unlike `vox_engine::test_util::Rig`'s hand-ticked `ManualEngine`, which
    /// has no handle for a job service to poll). The loopback fixture (-20 dB, a 240-sample/5 ms
    /// unreported residual, -40 dBFS noise) is the engine-level success test's own fixture
    /// (`crates/engine/tests/punch.rs::calibration_measures_the_unreported_residual_and_verify_confirms_it`,
    /// AC-16/AC-18), reused here via the shared `test_util::plug_loopback_devices` (H-53) instead
    /// of a copy.
    fn live_loopback_engine() -> (Engine, EngineHandle, FakeDriver) {
        let fake = FakeBackend::new(3);
        let mut lb = Loopback::new(
            vox_engine::test_util::dac_key(),
            vox_engine::test_util::mic_key(),
        );
        lb.gain_db = -20.0;
        lb.residual_ns = 5_000_000; // 5 ms = 240 samples at 48 kHz
        lb.noise = Some((3, -40.0));
        vox_engine::test_util::plug_loopback_devices(&fake, None, Some(lb), 0.0);
        let driver = fake.spawn_driver(Duration::from_millis(1));
        let registry =
            Arc::new(Registry::with_factories(vox_modules::builtin_factories()).unwrap());
        let mut cfg = EngineConfig::new(Arc::new(fake), registry);
        cfg.prefs = DevicePrefs {
            input_device: Some("Mic".to_owned()),
            input_channel: 1,
            ..DevicePrefs::default()
        };
        let engine = Engine::start(cfg).unwrap();
        let handle = engine.handle();
        (engine, handle, driver)
    }

    /// `crate::recording::device_setup`, retried against the same transient device-poll window as
    /// `nr_capture.rs`'s `retry_prepare` (the fake output/input open asynchronously).
    fn retry_device_setup(handle: &EngineHandle) -> crate::recording::DeviceSetup {
        for _ in 0..500 {
            if let Some(setup) = crate::recording::device_setup(handle) {
                return setup;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!("the fake devices never resolved a device setup");
    }

    #[derive(Debug, Clone, PartialEq)]
    enum RunEvent {
        Progress { state: JobState, fraction: f32 },
        Result(CalibrationResultDto),
        Notice(String),
    }

    /// Collects `events` until a terminal progress event lands (or a generous real-time timeout —
    /// a full run is about 10 s: 5 sweeps plus spacing, SPEC-022 §2.14/§4.7).
    fn run_to_terminal(events: &StdMutex<Vec<RunEvent>>) -> Vec<RunEvent> {
        for _ in 0..600 {
            {
                let got = events.lock().unwrap();
                if got.iter().any(|e| {
                    matches!(
                        e,
                        RunEvent::Progress {
                            state: JobState::Done | JobState::Failed | JobState::Cancelled,
                            ..
                        }
                    )
                }) {
                    return got.clone();
                }
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        panic!("the calibration run never reached a terminal progress event");
    }

    /// H-53 (SPEC-022 §2.14, AC-16/AC-18 at the job-service layer): a real engine's accepted
    /// calibration run posts its `Result` before the terminal `Done` progress event — already
    /// correctly ordered by inspection per the `watch` comment above, this is the missing test —
    /// with a plausible measured offset; applying it the way the wizard's Apply button does
    /// (`record_offset_set`'s own pieces, `device_setup` + `upsert_record_offset` — src-tauri is a
    /// thin command layer, so the business logic is what's tested, not the `#[tauri::command]`
    /// itself) lands the offset in Settings for the current device setup, readable back through
    /// `record_offset_for`/`current_offset_ms` (SPEC-022 §2.13).
    #[test]
    #[allow(clippy::float_cmp)] // exact round-tripped value, not a tolerance comparison
    fn a_successful_run_reports_the_result_before_done_and_the_offset_reaches_settings() {
        let (_engine, handle, _driver) = live_loopback_engine();
        let setup = retry_device_setup(&handle);

        let events: Arc<StdMutex<Vec<RunEvent>>> = Arc::new(StdMutex::new(Vec::new()));
        let sink = Arc::clone(&events);
        let emit: CalibrationEmitter = Arc::new(move |event| {
            let e = match event {
                CalibrationEvent::Progress(dto) => RunEvent::Progress {
                    state: dto.state,
                    fraction: dto.fraction,
                },
                CalibrationEvent::Result(r) => RunEvent::Result(r),
                CalibrationEvent::Notice(n) => RunEvent::Notice(n.key),
            };
            sink.lock().unwrap().push(e);
        });
        let service = CalibrationService::new(handle.clone(), emit);
        service.run(None).expect("a calibration run starts");

        let got = run_to_terminal(&events);
        let done = got
            .iter()
            .position(|e| {
                matches!(
                    e,
                    RunEvent::Progress {
                        state: JobState::Done,
                        ..
                    }
                )
            })
            .unwrap_or_else(|| panic!("the run did not finish as Done: {got:?}"));
        let result_idx = got
            .iter()
            .position(|e| matches!(e, RunEvent::Result(_)))
            .unwrap_or_else(|| panic!("no Result event: {got:?}"));
        assert!(
            result_idx < done,
            "Result must be posted before the terminal Done: {got:?}"
        );

        let RunEvent::Result(result) = &got[result_idx] else {
            unreachable!()
        };
        assert!(result.accepted, "{result:?}");
        assert!(
            result.offset_ms > 0.0 && result.offset_ms < 20.0,
            "a plausible measured offset (~5 ms expected): {result:?}"
        );

        // What the wizard's Apply button does (`record_offset_set`), using its own pieces
        // directly rather than the tauri command itself (CLAUDE.md: src-tauri is a thin command
        // layer with no business logic to bypass).
        let mut settings = Settings::default();
        crate::settings::upsert_record_offset(
            &mut settings.record_offsets,
            RecordOffsetEntry {
                host: setup.host.clone(),
                input_device: setup.input_device.clone(),
                output_device: setup.output_device.clone(),
                device_rate_hz: setup.device_rate_hz,
                offset_ms: result.offset_ms,
                source: RecordOffsetSource::Calibrated,
                updated_unix_ms: 1,
                confidence: Some(result.confidence),
                buffer_frames: setup.buffer_frames,
            },
        );

        let entry = crate::recording::record_offset_for(&setup, &settings.record_offsets)
            .expect("the measured offset reaches Settings for this device setup");
        assert_eq!(entry.offset_ms, result.offset_ms);
        assert_eq!(entry.source, RecordOffsetSource::Calibrated);
        assert_eq!(
            crate::recording::current_offset_ms(&handle, &settings),
            result.offset_ms,
            "the record panel's readout sees it too"
        );
    }
}

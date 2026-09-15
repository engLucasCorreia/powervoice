//! Normalize job (H-09): SPEC-010 peak normalize (S2-02) and LUFS normalize (S4-01) run as
//! cancellable jobs reporting `job_progress` (ADR-003), mirroring export.rs/nr_capture.rs/
//! loudness.rs's job-service pattern (MEMORY.md S4-04).
//!
//! Unlike those jobs, a normalize job *does* eventually write to the document, so the "busy"
//! state and the final commit live on `DocumentService` itself (`begin_normalize_job`/
//! `finish_normalize_peak`/`finish_normalize_lufs`, H-09) rather than here: this service only
//! drives `vox_project::plan_normalize_peak`/`plan_normalize_lufs` on its own thread in between
//! those two calls, so the job thread never holds the document lock while scanning/writing
//! (SPEC-010 §2.9's 60-min/10 s budget would starve every other command otherwise). Cancelling
//! (`cancel_job`) sets the same [`vox_project::CancelToken`] both passes already check — pass 2's
//! is the one a [`vox_project::ChunkWriter`] polls on every `append` (crate::store::writer), so
//! cancellation latency is bounded by one chunk (≤ 65 536 samples), matching SPEC-010 §2.8's
//! ≤ 100 ms.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use tauri::{AppHandle, Emitter as _, Runtime};
use vox_project::{CancelToken, plan_normalize_lufs, plan_normalize_peak, target_pct_to_db};

use crate::document::{
    DocumentService, NormalizeJobSource, NormalizeLufsNotice, NormalizeNotice, document_error,
};
use crate::ipc::document_dto::{DocumentDto, HistoryStateDto};
use crate::ipc::loudness_dto::normalize_lufs_notice_key;
use crate::ipc::{
    EventName, IpcError, IpcErrorCode, JobKind, JobProgressDto, JobState, NormalizeResultDto,
    Notice, NoticeLevel, emit_job_progress, emit_normalize_result, emit_notice,
};

// A handful of events per normalize job, moved once into the emitter — the `DocumentChanged`
// variant's size (DocumentDto grew with H-12's view state and T-301's `recovered`) doesn't
// matter here, so no boxing.
#[allow(clippy::large_enum_variant)]
enum NormalizeEvent {
    Progress(JobProgressDto),
    Result(NormalizeResultDto),
    Notice(Notice),
    /// A successful commit also refreshes the waveform/title bar and the Edit menu (mirrors
    /// `ipc::document_commands::after_edit`, reused here in event form since the job runs off
    /// the async command's own thread).
    DocumentChanged {
        info: DocumentDto,
        history: HistoryStateDto,
    },
}

type NormalizeEmitter = Arc<dyn Fn(NormalizeEvent) + Send + Sync>;

struct JobHandle {
    cancel: CancelToken,
}

struct Inner {
    documents: DocumentService,
    emit: NormalizeEmitter,
    next_id: AtomicU32,
    jobs: Mutex<HashMap<u32, JobHandle>>,
}

/// Cheaply cloneable (an `Arc` inside), managed as Tauri state like `ExportService`.
#[derive(Clone)]
pub struct NormalizeService(Arc<Inner>);

/// Builds the service for the running app (composition root, like `export::start`).
pub fn start<R: Runtime>(
    app: AppHandle<R>,
    documents: DocumentService,
) -> anyhow::Result<NormalizeService> {
    let emit: NormalizeEmitter = Arc::new(move |event| forward(&app, event));
    Ok(NormalizeService(Arc::new(Inner {
        documents,
        emit,
        next_id: AtomicU32::new(1),
        jobs: Mutex::new(HashMap::new()),
    })))
}

fn forward<R: Runtime>(app: &AppHandle<R>, event: NormalizeEvent) {
    let result = match event {
        NormalizeEvent::Progress(dto) => emit_job_progress(app, dto),
        NormalizeEvent::Result(dto) => emit_normalize_result(app, dto),
        NormalizeEvent::Notice(notice) => emit_notice(app, notice),
        NormalizeEvent::DocumentChanged { info, history } => app
            .emit(EventName::document_changed.as_str(), info)
            .and_then(|()| app.emit(EventName::history_state.as_str(), history)),
    };
    if let Err(error) = result {
        tracing::warn!(%error, "emitting a normalize event failed");
    }
}

/// SPEC-010 §2.4's IPC contract: "exactly one of the two targets".
fn resolve_target_db(target_db: Option<f64>, target_pct: Option<f64>) -> Result<f64, IpcError> {
    match (target_db, target_pct) {
        (Some(db), None) => Ok(db),
        (None, Some(pct)) => Ok(target_pct_to_db(pct)),
        _ => Err(IpcError::new(
            IpcErrorCode::InvalidArgument,
            "error.invalid_argument",
        )),
    }
}

fn notice_from_error(err: &IpcError) -> Notice {
    let mut notice = Notice::toast(NoticeLevel::Error, err.key.clone());
    for (name, value) in &err.params {
        notice = notice.with_param(name.clone(), value.clone());
    }
    notice
}

impl NormalizeService {
    /// `edit_normalize_peak_start` (S2-02, H-09): resolves the dB/% target, validates the scope
    /// via `DocumentService::begin_normalize_job` (also marks the document busy), then runs the
    /// scan+write job on its own thread. Returns the job id immediately.
    pub fn start_peak_job(
        &self,
        start: u64,
        end: u64,
        target_db: Option<f64>,
        target_pct: Option<f64>,
    ) -> Result<u32, IpcError> {
        let target_db = resolve_target_db(target_db, target_pct)?;
        DocumentService::validate_normalize_peak_target(target_db)?;
        let source = self.0.documents.begin_normalize_job(start, end)?;
        let job_id = self.0.next_id.fetch_add(1, Ordering::Relaxed);
        let cancel = CancelToken::new();
        self.0.jobs.lock().unwrap().insert(
            job_id,
            JobHandle {
                cancel: cancel.clone(),
            },
        );
        let inner = Arc::clone(&self.0);
        std::thread::Builder::new()
            .name("normalize-peak-job".into())
            .spawn(move || run_peak_job(inner, job_id, source, target_db, cancel))
            .map_err(|e| IpcError::internal(e.to_string()))?;
        Ok(job_id)
    }

    /// `edit_normalize_lufs_start` (S4-01, H-09): mirrors [`Self::start_peak_job`] (LUFS has no %
    /// mode, SPEC-010 §2.4/M6 scope).
    pub fn start_lufs_job(&self, start: u64, end: u64, target_lufs: f64) -> Result<u32, IpcError> {
        DocumentService::validate_normalize_lufs_target(target_lufs)?;
        let source = self.0.documents.begin_normalize_job(start, end)?;
        let job_id = self.0.next_id.fetch_add(1, Ordering::Relaxed);
        let cancel = CancelToken::new();
        self.0.jobs.lock().unwrap().insert(
            job_id,
            JobHandle {
                cancel: cancel.clone(),
            },
        );
        let inner = Arc::clone(&self.0);
        std::thread::Builder::new()
            .name("normalize-lufs-job".into())
            .spawn(move || run_lufs_job(inner, job_id, source, target_lufs, cancel))
            .map_err(|e| IpcError::internal(e.to_string()))?;
        Ok(job_id)
    }

    /// `edit_normalize_peak_cancel`/`edit_normalize_lufs_cancel`: best-effort, like
    /// `ExportService::cancel_job`. A no-op for an unknown or already-finished job id.
    pub fn cancel_job(&self, job_id: u32) {
        if let Some(job) = self.0.jobs.lock().unwrap().get(&job_id) {
            job.cancel.cancel();
        }
    }
}

fn run_peak_job(
    inner: Arc<Inner>,
    job_id: u32,
    source: NormalizeJobSource,
    target_db: f64,
    cancel: CancelToken,
) {
    let emit = Arc::clone(&inner.emit);
    let plan_result = plan_normalize_peak(
        &source.store,
        &source.snapshot,
        source.sample_rate_hz,
        source.range,
        target_db,
        &cancel,
        |fraction| {
            emit(NormalizeEvent::Progress(JobProgressDto {
                job_id,
                kind: JobKind::NormalizePeak,
                state: JobState::Running,
                fraction,
            }));
        },
    );
    inner.jobs.lock().unwrap().remove(&job_id);

    match plan_result {
        Ok(plan) => match inner.documents.finish_normalize_peak(&source, plan) {
            Ok(outcome) => {
                (inner.emit)(NormalizeEvent::Progress(JobProgressDto {
                    job_id,
                    kind: JobKind::NormalizePeak,
                    state: JobState::Done,
                    fraction: 1.0,
                }));
                (inner.emit)(NormalizeEvent::DocumentChanged {
                    info: inner.documents.info().into(),
                    history: inner.documents.history_state().into(),
                });
                if let Some(notice) = outcome.notice {
                    let key = match notice {
                        NormalizeNotice::Silent => "notice.normalize_silent",
                        NormalizeNotice::AlreadyNormalized => "notice.normalize_already",
                    };
                    (inner.emit)(NormalizeEvent::Notice(Notice::toast(
                        NoticeLevel::Info,
                        key,
                    )));
                }
                (inner.emit)(NormalizeEvent::Result(NormalizeResultDto {
                    job_id,
                    kind: JobKind::NormalizePeak,
                    result: outcome.result.into(),
                }));
            }
            Err(err) => fail_job(&inner, job_id, JobKind::NormalizePeak, err),
        },
        Err(project_err) => {
            inner.documents.abandon_normalize_job();
            fail_job(
                &inner,
                job_id,
                JobKind::NormalizePeak,
                document_error(project_err),
            );
        }
    }
}

fn run_lufs_job(
    inner: Arc<Inner>,
    job_id: u32,
    source: NormalizeJobSource,
    target_lufs: f64,
    cancel: CancelToken,
) {
    let emit = Arc::clone(&inner.emit);
    let plan_result = plan_normalize_lufs(
        &source.store,
        &source.snapshot,
        source.sample_rate_hz,
        source.range,
        target_lufs,
        &cancel,
        |fraction| {
            emit(NormalizeEvent::Progress(JobProgressDto {
                job_id,
                kind: JobKind::NormalizeLufs,
                state: JobState::Running,
                fraction,
            }));
        },
    );
    inner.jobs.lock().unwrap().remove(&job_id);

    match plan_result {
        Ok(plan) => match inner.documents.finish_normalize_lufs(&source, plan) {
            Ok(outcome) => {
                (inner.emit)(NormalizeEvent::Progress(JobProgressDto {
                    job_id,
                    kind: JobKind::NormalizeLufs,
                    state: JobState::Done,
                    fraction: 1.0,
                }));
                (inner.emit)(NormalizeEvent::DocumentChanged {
                    info: inner.documents.info().into(),
                    history: inner.documents.history_state().into(),
                });
                if let Some(notice) = outcome.notice {
                    let key = normalize_lufs_notice_key(notice);
                    let mut event = Notice::toast(NoticeLevel::Info, key);
                    if let NormalizeLufsNotice::TruePeakCeilingExceeded {
                        predicted_true_peak_dbtp,
                    } = notice
                    {
                        event = event.with_param("value", format!("{predicted_true_peak_dbtp:.1}"));
                    }
                    (inner.emit)(NormalizeEvent::Notice(event));
                }
                (inner.emit)(NormalizeEvent::Result(NormalizeResultDto {
                    job_id,
                    kind: JobKind::NormalizeLufs,
                    result: outcome.result.into(),
                }));
            }
            Err(err) => fail_job(&inner, job_id, JobKind::NormalizeLufs, err),
        },
        Err(project_err) => {
            inner.documents.abandon_normalize_job();
            fail_job(
                &inner,
                job_id,
                JobKind::NormalizeLufs,
                document_error(project_err),
            );
        }
    }
}

/// Common tail for a cancelled or failed job (either pass 1/2's own `ProjectError::Cancelled`, or
/// a conflict `DocumentService::finish_normalize_peak`/`finish_normalize_lufs` detected):
/// `job_progress` reports `Cancelled`/`Failed`, and a cancellation posts no notice (the user asked
/// for it) while a real failure does. H-30: the notice goes out before the terminal progress
/// event (bake.rs's `fail_job` convention), so anything that sees `Failed` — the UI's job store,
/// tests — already has the reason.
fn fail_job(inner: &Inner, job_id: u32, kind: JobKind, err: IpcError) {
    let state = if err.code == IpcErrorCode::Cancelled {
        JobState::Cancelled
    } else {
        JobState::Failed
    };
    if state == JobState::Failed {
        (inner.emit)(NormalizeEvent::Notice(notice_from_error(&err)));
    }
    (inner.emit)(NormalizeEvent::Progress(JobProgressDto {
        job_id,
        kind,
        state,
        fraction: 0.0,
    }));
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::Duration;

    use vox_engine::backend::fake::FakeBackend;
    use vox_engine::{Engine, EngineConfig};
    use vox_rack::Registry;

    use super::*;
    use crate::document::DocumentService;

    fn tmp_dir(tag: &str) -> PathBuf {
        crate::test_util::tmp_dir(&format!("normalize-service-{tag}"))
    }

    fn test_engine() -> Engine {
        let fake = FakeBackend::new(1);
        let registry =
            Arc::new(Registry::with_factories(vox_modules::builtin_factories()).unwrap());
        Engine::start(EngineConfig::new(Arc::new(fake), registry)).unwrap()
    }

    fn open_test_doc(documents: &DocumentService, dir: &std::path::Path, samples: &[f32]) {
        let path = dir.join("in.wav");
        vox_testkit::wav::write_wav_file(
            &path,
            samples,
            1,
            48_000,
            vox_testkit::wav::BitDepth::Float32,
        )
        .unwrap();
        documents.open(&path, false).unwrap();
    }

    /// A snapshot of one `NormalizeEvent`, captured by a test-only emitter (mirrors `export.rs`'s
    /// own tests, which build `Inner` directly with a closure instead of a real `AppHandle`).
    #[derive(Debug, Clone)]
    enum TestEvent {
        Progress { fraction: f32, state: JobState },
        Result(NormalizeResultDto),
        Notice(String),
        DocumentChanged,
    }

    impl From<NormalizeEvent> for TestEvent {
        fn from(event: NormalizeEvent) -> Self {
            match event {
                NormalizeEvent::Progress(dto) => TestEvent::Progress {
                    fraction: dto.fraction,
                    state: dto.state,
                },
                NormalizeEvent::Result(dto) => TestEvent::Result(dto),
                NormalizeEvent::Notice(notice) => TestEvent::Notice(notice.key),
                NormalizeEvent::DocumentChanged { .. } => TestEvent::DocumentChanged,
            }
        }
    }

    /// A `NormalizeService` over a fresh document, with a test-only emitter that records every
    /// event instead of needing a real Tauri `AppHandle`.
    fn service_with_doc(
        tag: &str,
        samples: &[f32],
    ) -> (
        NormalizeService,
        DocumentService,
        Engine,
        PathBuf,
        Arc<Mutex<Vec<TestEvent>>>,
    ) {
        let dir = tmp_dir(tag);
        let engine = test_engine();
        let documents = DocumentService::new(dir.join("sessions"), engine.handle());
        open_test_doc(&documents, &dir, samples);

        let events: Arc<Mutex<Vec<TestEvent>>> = Arc::new(Mutex::new(Vec::new()));
        let events_for_emit = Arc::clone(&events);
        let emit: NormalizeEmitter = Arc::new(move |event| {
            events_for_emit.lock().unwrap().push(TestEvent::from(event));
        });
        let inner = Arc::new(Inner {
            documents: documents.clone(),
            emit,
            next_id: AtomicU32::new(1),
            jobs: Mutex::new(HashMap::new()),
        });
        (NormalizeService(inner), documents, engine, dir, events)
    }

    /// Waits (polling, like `export.rs`'s own live-engine tests) until `events` contains a
    /// `Progress` entry whose state is no longer `Running`, and returns it.
    fn wait_for_finish(events: &Mutex<Vec<TestEvent>>) -> JobState {
        for _ in 0..500 {
            if let Some(state) = events.lock().unwrap().iter().find_map(|e| match e {
                TestEvent::Progress { state, .. } if *state != JobState::Running => Some(*state),
                _ => None,
            }) {
                return state;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("normalize job never reported a terminal state within 5s");
    }

    #[test]
    fn start_peak_job_runs_end_to_end_and_reports_done_with_a_result() {
        let samples = vox_testkit::signal::sine(440.0, -12.0, 0.2, 48_000).unwrap();
        let len = samples.len() as u64;
        let (service, documents, _engine, dir, events) = service_with_doc("peak-e2e", &samples);

        let job_id = service.start_peak_job(0, len, Some(-1.0), None).unwrap();
        assert_eq!(wait_for_finish(&events), JobState::Done);
        assert!(!documents.is_normalize_busy());

        let got = events.lock().unwrap().clone();
        let result = got.iter().find_map(|e| match e {
            TestEvent::Result(dto) if dto.job_id == job_id => Some(*dto),
            _ => None,
        });
        let result = result.expect("expected a normalize_result event");
        assert!(result.result.changed);
        assert!(
            got.iter().any(|e| matches!(e, TestEvent::DocumentChanged)),
            "a successful commit refreshes document_changed/history_state"
        );
        assert!(
            got.iter().any(|e| matches!(
                e,
                TestEvent::Progress { fraction, state: JobState::Done }
                    if (*fraction - 1.0).abs() < 1e-6
            )),
            "the Done progress event reports fraction 1.0"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A no-op normalize (silent scope) posts `notice.normalize_silent` as a `Notice` event and
    /// still reports `Done` with a `normalize_result` whose `changed` is `false`.
    #[test]
    fn start_peak_job_on_a_silent_scope_posts_a_notice_and_reports_a_no_op_result() {
        let samples = vec![0.0f32; 1_000];
        let (service, documents, _engine, dir, events) = service_with_doc("peak-silent", &samples);

        let job_id = service.start_peak_job(0, 1_000, Some(-1.0), None).unwrap();
        assert_eq!(wait_for_finish(&events), JobState::Done);
        assert!(!documents.is_normalize_busy());

        let got = events.lock().unwrap().clone();
        assert!(
            got.iter()
                .any(|e| matches!(e, TestEvent::Notice(key) if key == "notice.normalize_silent")),
        );
        let result = got
            .iter()
            .find_map(|e| match e {
                TestEvent::Result(dto) if dto.job_id == job_id => Some(*dto),
                _ => None,
            })
            .expect("a no-op still reports a normalize_result");
        assert!(!result.result.changed);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn start_peak_job_accepts_a_percent_target() {
        let samples = vox_testkit::signal::sine(440.0, -12.0, 0.2, 48_000).unwrap();
        let len = samples.len() as u64;
        let (service, _documents, _engine, dir, events) = service_with_doc("peak-pct", &samples);

        service.start_peak_job(0, len, None, Some(50.0)).unwrap();
        assert_eq!(wait_for_finish(&events), JobState::Done);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn start_peak_job_rejects_neither_or_both_targets() {
        let samples = vox_testkit::signal::sine(440.0, -12.0, 0.2, 48_000).unwrap();
        let len = samples.len() as u64;
        let (service, documents, _engine, dir, _events) = service_with_doc("peak-target", &samples);

        let err = service.start_peak_job(0, len, None, None).unwrap_err();
        assert_eq!(err.code, IpcErrorCode::InvalidArgument);
        let err = service
            .start_peak_job(0, len, Some(-1.0), Some(50.0))
            .unwrap_err();
        assert_eq!(err.code, IpcErrorCode::InvalidArgument);
        assert!(
            !documents.is_normalize_busy(),
            "an invalid target is rejected before the busy flag is set"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn start_lufs_job_runs_end_to_end_and_reports_done_with_a_result() {
        let samples = vox_testkit::signal::sine(1000.0, -20.0, 2.0, 48_000).unwrap();
        let len = samples.len() as u64;
        let (service, documents, _engine, dir, events) = service_with_doc("lufs-e2e", &samples);

        let job_id = service.start_lufs_job(0, len, -19.0).unwrap();
        assert_eq!(wait_for_finish(&events), JobState::Done);
        assert!(!documents.is_normalize_busy());

        // `normalize_result` is a separate emit just after the terminal `job_progress`, so under
        // load it can land a moment later — poll for it instead of racing it.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let result = loop {
            let found = events.lock().unwrap().iter().find_map(|e| match e {
                TestEvent::Result(dto) if dto.job_id == job_id => Some(*dto),
                _ => None,
            });
            if let Some(dto) = found {
                break dto;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "expected a normalize_result event"
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        };
        assert!(result.result.changed);
        assert_eq!(result.kind, JobKind::NormalizeLufs);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// H-09 ticket AC: "cancel mid-job leaves hash unchanged" — exercised here through the whole
    /// service (real thread, `cancel_job`), not just `DocumentService`'s own contract
    /// (`document.rs`'s `normalize_peak_cancel_mid_job_leaves_the_document_untouched`).
    #[test]
    fn cancel_job_reports_cancelled_and_leaves_the_document_untouched() {
        // A longer, louder signal so pass 2 (the write) is still running when we cancel.
        let samples = vox_testkit::signal::sine(440.0, -12.0, 3.0, 48_000).unwrap();
        let len = samples.len() as u64;
        let (service, documents, _engine, dir, events) = service_with_doc("cancel", &samples);
        let before_rev = documents.info().audio_rev;

        let job_id = service.start_peak_job(0, len, Some(-1.0), None).unwrap();
        service.cancel_job(job_id);
        assert_eq!(wait_for_finish(&events), JobState::Cancelled);
        assert!(
            !documents.is_normalize_busy(),
            "cancel clears the busy flag"
        );

        let got = events.lock().unwrap().clone();
        assert!(
            !got.iter().any(|e| matches!(e, TestEvent::Result(_))),
            "a cancelled job never reports a normalize_result"
        );
        assert_eq!(documents.info().audio_rev, before_rev, "rev unchanged");
        assert!(!documents.history_state().can_undo, "no undo entry");

        // The busy flag being clear again means a normal normalize still works afterwards.
        events.lock().unwrap().clear();
        service.start_peak_job(0, len, Some(-1.0), None).unwrap();
        assert_eq!(wait_for_finish(&events), JobState::Done);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_second_job_is_refused_while_one_is_running() {
        let samples = vox_testkit::signal::sine(440.0, -12.0, 2.0, 48_000).unwrap();
        let len = samples.len() as u64;
        let (service, documents, _engine, dir, events) = service_with_doc("busy", &samples);

        let job_id = service.start_peak_job(0, len, Some(-1.0), None).unwrap();
        let err = service
            .start_peak_job(0, len, Some(-1.0), None)
            .unwrap_err();
        assert_eq!(err.code, IpcErrorCode::Busy);

        service.cancel_job(job_id);
        assert_eq!(wait_for_finish(&events), JobState::Cancelled);
        assert!(!documents.is_normalize_busy());

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// H-30: a failed job's error notice is already posted by the time `wait_for_finish` sees the
    /// terminal `Failed` state — checked without any extra polling for the notice itself (the
    /// point of the ordering fix). Replacing the document deterministically fails the job with
    /// `error.document_busy` (`finish_normalize_peak` sees a session mismatch): the swap happens
    /// from inside the job's own first progress callback (`plan_normalize_peak`'s `on_progress
    /// (0.0)`, called before any scanning) — the same "hook a callback instead of racing wall
    /// clock time" trick `plan_normalize_peak_cancel_mid_write_returns_cancelled` uses for
    /// cancellation, so this needs no timing margin at all.
    #[test]
    fn a_failed_job_posts_the_notice_before_the_terminal_failed_progress_event() {
        let samples = vox_testkit::signal::sine(440.0, -12.0, 0.2, 48_000).unwrap();
        let len = samples.len() as u64;
        let dir = tmp_dir("replaced");
        let engine = test_engine();
        let documents = DocumentService::new(dir.join("sessions"), engine.handle());
        open_test_doc(&documents, &dir, &samples);

        let other_path = dir.join("other.wav");
        vox_testkit::wav::write_wav_file(
            &other_path,
            &vox_testkit::signal::sine(220.0, -12.0, 0.05, 48_000).unwrap(),
            1,
            48_000,
            vox_testkit::wav::BitDepth::Float32,
        )
        .unwrap();

        let events: Arc<Mutex<Vec<TestEvent>>> = Arc::new(Mutex::new(Vec::new()));
        let events_for_emit = Arc::clone(&events);
        let documents_for_emit = documents.clone();
        let swapped = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let emit: NormalizeEmitter = Arc::new(move |event| {
            if let NormalizeEvent::Progress(dto) = &event
                && dto.state == JobState::Running
                && !swapped.swap(true, Ordering::SeqCst)
            {
                documents_for_emit.open(&other_path, false).unwrap();
            }
            events_for_emit.lock().unwrap().push(TestEvent::from(event));
        });
        let inner = Arc::new(Inner {
            documents: documents.clone(),
            emit,
            next_id: AtomicU32::new(1),
            jobs: Mutex::new(HashMap::new()),
        });
        let service = NormalizeService(inner);

        service.start_peak_job(0, len, Some(-1.0), None).unwrap();
        assert_eq!(wait_for_finish(&events), JobState::Failed);
        assert!(!documents.is_normalize_busy());

        let got = events.lock().unwrap().clone();
        assert!(
            got.iter()
                .any(|e| matches!(e, TestEvent::Notice(key) if key == "error.document_busy")),
            "the notice must already be present once `Failed` is observed: {got:?}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}

//! Bake rack job (T-602): renders the **live rack** into the selection or the whole document as
//! one undoable `history.bake` entry, then resets the rack (SPEC-004 §2.2, OD-4 default). Mirrors
//! `normalize.rs`'s write-back job pattern (MEMORY.md H-09):
//!
//! 1. [`BakeService::start_job`] snapshots the live rack (`EngineHandle::rack_model()`) and
//!    marks the document busy through `DocumentService::begin_bake_job` (the normalize busy flag:
//!    one document job at a time; edits and undo/redo answer `error.document_busy`);
//! 2. the job thread runs `vox_engine::bake::plan_bake` — the render path export uses too, so the
//!    bake is bit-identical to an export of the same range — without holding the document lock,
//!    reporting `job_progress` (kind `bake`, ≤ 10 Hz) and holding a spectrogram
//!    `BackgroundJob` so tile work uses at most half the workers (SPEC-007 §4.1);
//! 3. `DocumentService::finish_bake` commits the edit only if the document is still the one the
//!    job read, then loads the reset rack. The entry's attachment carries the pre-bake rack, so
//!    Undo restores audio *and* rack, and Redo re-applies both (SPEC-004 AC-16).
//!
//! Cancelling, a failing or crashing slot (ADR-008 §5: offline renders abort) or a missing module
//! abort the job with the document and the rack exactly as they were.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tauri::{AppHandle, Emitter as _, Runtime};
use vox_engine::EngineHandle;
use vox_engine::bake::{DocumentRange, RangeRenderError, plan_bake};
use vox_engine::spectro::SpectroService;
use vox_project::CancelToken;
use vox_rack::offline::RenderError;
use vox_rack::{RackModel, Registry};

use crate::document::{DocumentService, NormalizeJobSource, document_error};
use crate::ipc::document_dto::{DocumentDto, HistoryStateDto};
use crate::ipc::{
    EventName, IpcError, IpcErrorCode, JobKind, JobProgressDto, JobState, Notice, NoticeLevel,
    emit_job_progress, emit_notice,
};

/// ADR-003: at most ~10 `job_progress` events per second per job.
const PROGRESS_INTERVAL: Duration = Duration::from_millis(100);

// A handful of events per job, moved once into the emitter (see `normalize.rs`).
#[allow(clippy::large_enum_variant)]
enum BakeEvent {
    Progress(JobProgressDto),
    Notice(Notice),
    /// A committed bake refreshes the waveform/title bar and the Edit menu.
    DocumentChanged {
        info: DocumentDto,
        history: HistoryStateDto,
    },
}

type BakeEmitter = Arc<dyn Fn(BakeEvent) + Send + Sync>;

struct Inner {
    documents: DocumentService,
    /// The live rack snapshot source, read synchronously in `start_job` (like export, H-08).
    engine: EngineHandle,
    registry: Arc<Registry>,
    /// SPEC-007 §4.1: halves tile work while a bake runs (`None` in tests that don't need it).
    spectro: Option<Arc<SpectroService>>,
    emit: BakeEmitter,
    next_id: AtomicU32,
    jobs: Mutex<HashMap<u32, CancelToken>>,
}

/// Cheaply cloneable (an `Arc` inside), managed as Tauri state like `NormalizeService`.
#[derive(Clone)]
pub struct BakeService(Arc<Inner>);

/// Builds the service for the running app (composition root: its own registry instance, like
/// `export::start`).
pub fn start<R: Runtime>(
    app: AppHandle<R>,
    documents: DocumentService,
    engine: EngineHandle,
    spectro: Arc<SpectroService>,
) -> anyhow::Result<BakeService> {
    let registry = crate::plugins::registry()?;
    let emit: BakeEmitter = Arc::new(move |event| forward(&app, event));
    Ok(BakeService(Arc::new(Inner {
        documents,
        engine,
        registry,
        spectro: Some(spectro),
        emit,
        next_id: AtomicU32::new(1),
        jobs: Mutex::new(HashMap::new()),
    })))
}

fn forward<R: Runtime>(app: &AppHandle<R>, event: BakeEvent) {
    let result = match event {
        BakeEvent::Progress(dto) => emit_job_progress(app, dto),
        BakeEvent::Notice(notice) => emit_notice(app, notice),
        BakeEvent::DocumentChanged { info, history } => app
            .emit(EventName::document_changed.as_str(), info)
            .and_then(|()| app.emit(EventName::history_state.as_str(), history)),
    };
    if let Err(error) = result {
        tracing::warn!(%error, "emitting a bake event failed");
    }
}

/// Whether `model` has anything to bake: at least one slot that isn't bypassed (Effects → Bake
/// Rack is disabled otherwise). Whole-rack A/B is not part of the model — offline renders ignore
/// it (SPEC-012 §2.3).
pub fn rack_is_active(model: &RackModel) -> bool {
    model.slots.iter().any(|s| !s.bypass)
}

fn rack_inactive() -> IpcError {
    IpcError::new(IpcErrorCode::InvalidArgument, "error.bake.rack_inactive")
}

fn bake_error(err: RangeRenderError) -> IpcError {
    match err {
        RangeRenderError::Cancelled => IpcError::new(IpcErrorCode::Cancelled, "error.cancelled"),
        RangeRenderError::Render(RenderError::SlotFailed { name, .. }) => {
            IpcError::new(IpcErrorCode::Internal, "error.bake.slot_failed").with_param("slot", name)
        }
        RangeRenderError::Render(RenderError::MissingModules(ids)) => {
            IpcError::new(IpcErrorCode::InvalidArgument, "error.bake.missing_modules")
                .with_param("modules", ids.join(", "))
        }
        RangeRenderError::Render(other) => IpcError::new(IpcErrorCode::Internal, "error.bake.rack")
            .with_param("message", other.to_string()),
        RangeRenderError::Project(err) => document_error(err),
    }
}

fn notice_from_error(err: &IpcError) -> Notice {
    let mut notice = Notice::toast(NoticeLevel::Error, err.key.clone());
    for (name, value) in &err.params {
        notice = notice.with_param(name.clone(), value.clone());
    }
    notice
}

impl BakeService {
    /// `edit_bake_start`: snapshots the live rack, refuses an inactive rack
    /// (`error.bake.rack_inactive`), validates the scope and marks the document busy
    /// (`DocumentService::begin_bake_job`), then runs the job on its own thread. Returns the job
    /// id immediately.
    pub fn start_job(&self, start: u64, end: u64) -> Result<u32, IpcError> {
        // `None` only while the app shuts down: nothing to bake then.
        let model = self.0.engine.rack_model().unwrap_or_default();
        if !rack_is_active(&model) {
            return Err(rack_inactive());
        }
        let source = self.0.documents.begin_bake_job(start, end)?;
        let job_id = self.0.next_id.fetch_add(1, Ordering::Relaxed);
        let cancel = CancelToken::new();
        self.0.jobs.lock().unwrap().insert(job_id, cancel.clone());
        let inner = Arc::clone(&self.0);
        let spawned = std::thread::Builder::new()
            .name("bake-job".into())
            .spawn(move || run_job(inner, job_id, source, model, cancel));
        if let Err(e) = spawned {
            self.0.jobs.lock().unwrap().remove(&job_id);
            self.0.documents.abandon_bake_job();
            return Err(IpcError::internal(e.to_string()));
        }
        Ok(job_id)
    }

    /// `edit_bake_cancel`: best-effort, like `NormalizeService::cancel_job` (takes effect within
    /// one 4096-frame render block). A no-op for an unknown or finished job id.
    pub fn cancel_job(&self, job_id: u32) {
        if let Some(cancel) = self.0.jobs.lock().unwrap().get(&job_id) {
            cancel.cancel();
        }
    }
}

fn progress_event(job_id: u32, state: JobState, fraction: f32) -> BakeEvent {
    BakeEvent::Progress(JobProgressDto {
        job_id,
        kind: JobKind::Bake,
        state,
        fraction,
    })
}

fn run_job(
    inner: Arc<Inner>,
    job_id: u32,
    source: NormalizeJobSource,
    model: RackModel,
    cancel: CancelToken,
) {
    // SPEC-007 §4.1: tiles use at most half of the workers while the bake runs.
    let background = inner.spectro.as_ref().map(|s| s.begin_background_job());
    let emit = Arc::clone(&inner.emit);
    let mut last_progress: Option<Instant> = None;
    let plan = plan_bake(
        &inner.registry,
        &model,
        DocumentRange {
            store: &source.store,
            snapshot: &source.snapshot,
            sample_rate_hz: source.sample_rate_hz,
            range: source.range,
        },
        &cancel,
        |fraction| {
            if last_progress.is_none_or(|t| t.elapsed() >= PROGRESS_INTERVAL) {
                last_progress = Some(Instant::now());
                emit(progress_event(job_id, JobState::Running, fraction));
            }
        },
    );
    drop(background);
    inner.jobs.lock().unwrap().remove(&job_id);

    match plan {
        // SPEC-004 OD-4 default: after the bake the rack is reset (empty).
        Ok(edit) => match inner
            .documents
            .finish_bake(&source, edit, RackModel::default())
        {
            Ok(_) => {
                (inner.emit)(progress_event(job_id, JobState::Done, 1.0));
                (inner.emit)(BakeEvent::DocumentChanged {
                    info: inner.documents.info().into(),
                    history: inner.documents.history_state().into(),
                });
                (inner.emit)(BakeEvent::Notice(Notice::toast(
                    NoticeLevel::Info,
                    "notice.bake.done",
                )));
            }
            Err(err) => fail_job(&inner, job_id, &err),
        },
        Err(err) => {
            inner.documents.abandon_bake_job();
            fail_job(&inner, job_id, &bake_error(err));
        }
    }
}

/// A cancelled job reports `Cancelled` with no notice (the user asked for it); a failure
/// reports `Failed` plus an error notice naming the reason.
fn fail_job(inner: &Inner, job_id: u32, err: &IpcError) {
    let state = if err.code == IpcErrorCode::Cancelled {
        JobState::Cancelled
    } else {
        JobState::Failed
    };
    // The notice goes out before the terminal progress event, so anything that sees `Failed`
    // (the UI's job store, tests) already has the reason.
    if state == JobState::Failed {
        (inner.emit)(BakeEvent::Notice(notice_from_error(err)));
    }
    (inner.emit)(progress_event(job_id, state, 0.0));
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    use vox_engine::backend::fake::{FakeBackend, FakeDevice, FakeDirection, FakeDriver};
    use vox_engine::{Engine, EngineConfig, HostId};
    use vox_module_api::test_util::TestNaN;
    use vox_module_api::{
        Module, ModuleDescriptor, ModuleError, ModuleFactory, ModuleRef, ModuleState, Version,
    };
    use vox_modules::{Gain, NoiseReduction, ParametricEq};
    use vox_project::SnapshotReader;
    use vox_rack::SlotModel;

    use super::*;
    use crate::document::DocumentService;

    fn tmp_dir(tag: &str) -> PathBuf {
        crate::test_util::tmp_dir(&format!("bake-{tag}"))
    }

    /// A live engine on a fake output device (the rack only exists with an open output, like
    /// `export.rs::live_engine_with_gain_minus_6_db`).
    fn live_engine() -> (Engine, EngineHandle, FakeDriver) {
        let fake = FakeBackend::new(1);
        fake.plug(
            HostId::Alsa,
            FakeDevice::new("DAC").with_output(
                FakeDirection::new(2, &[48_000], 48_000)
                    .default_buffer(256)
                    .record_output(),
            ),
        );
        let driver = fake.spawn_driver(Duration::from_millis(1));
        let registry =
            Arc::new(Registry::with_factories(vox_modules::builtin_factories()).unwrap());
        let engine = Engine::start(EngineConfig::new(Arc::new(fake), registry)).unwrap();
        let handle = engine.handle();
        (engine, handle, driver)
    }

    /// Loads `model` into the live rack once the fake output is open.
    fn load_rack(handle: &EngineHandle, model: &RackModel) {
        for _ in 0..500 {
            if let Some(Ok(snapshot)) = handle.rack_load_model(model.clone())
                && snapshot.slots.len() == model.slots.len()
            {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!("the fake output device never opened a live rack");
    }

    fn slot(id: &str, bypass: bool, params: &[(&str, f64)], blob: Option<Vec<u8>>) -> SlotModel {
        SlotModel::new(
            &ModuleRef {
                id: id.into(),
                version: Version::new(1, 0, 0),
            },
            bypass,
            &ModuleState {
                format_version: 1,
                params: params
                    .iter()
                    .map(|(k, v)| ((*k).to_owned(), *v))
                    .collect::<BTreeMap<_, _>>(),
                blob,
            },
        )
    }

    /// Slots, parameter values, bypass flags and a state blob (SPEC-004 AC-16's rack
    /// description): Noise Reduction with a (verbatim-kept) blob and its latency, a bypassed
    /// Gain, an EQ band (a long tail) and an active Gain.
    fn ac16_rack() -> RackModel {
        RackModel {
            slots: vec![
                slot(
                    NoiseReduction::ID,
                    false,
                    &[("reduction_db", -12.0)],
                    Some(b"PVTEST-not-a-real-noise-print".to_vec()),
                ),
                slot(Gain::ID, true, &[("gain_db", 12.0)], None),
                slot(
                    ParametricEq::ID,
                    false,
                    &[("b2_on", 1.0), ("b2_gain_db", 6.5)],
                    None,
                ),
                slot(Gain::ID, false, &[("gain_db", -6.123456789012345)], None),
            ],
        }
    }

    #[derive(Debug, Clone, PartialEq)]
    enum TestEvent {
        Progress { state: JobState, fraction: f32 },
        Notice(String, Vec<(String, String)>),
        DocumentChanged,
    }

    struct Harness {
        service: BakeService,
        documents: DocumentService,
        handle: EngineHandle,
        events: Arc<Mutex<Vec<TestEvent>>>,
        dir: PathBuf,
        _engine: Engine,
        _driver: FakeDriver,
    }

    impl Drop for Harness {
        fn drop(&mut self) {
            let _ = self.documents.close();
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    fn harness(tag: &str, samples: &[f32], registry: Registry) -> Harness {
        let dir = tmp_dir(tag);
        let (engine, handle, driver) = live_engine();
        let documents = DocumentService::new(dir.join("sessions"), handle.clone());
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
        let events: Arc<Mutex<Vec<TestEvent>>> = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&events);
        let emit: BakeEmitter = Arc::new(move |event| {
            let e = match event {
                BakeEvent::Progress(dto) => TestEvent::Progress {
                    state: dto.state,
                    fraction: dto.fraction,
                },
                BakeEvent::Notice(n) => TestEvent::Notice(n.key, n.params.into_iter().collect()),
                BakeEvent::DocumentChanged { .. } => TestEvent::DocumentChanged,
            };
            sink.lock().unwrap().push(e);
        });
        let spectro = Arc::new(SpectroService::new(vox_engine::spectro::SpectroConfig {
            workers: 2,
            cache_cap_bytes: None,
        }));
        let service = BakeService(Arc::new(Inner {
            documents: documents.clone(),
            engine: handle.clone(),
            registry: Arc::new(registry),
            spectro: Some(spectro),
            emit,
            next_id: AtomicU32::new(1),
            jobs: Mutex::new(HashMap::new()),
        }));
        Harness {
            service,
            documents,
            handle,
            events,
            dir,
            _engine: engine,
            _driver: driver,
        }
    }

    fn builtins() -> Registry {
        Registry::with_factories(vox_modules::builtin_factories()).unwrap()
    }

    fn wait_for_finish(events: &Mutex<Vec<TestEvent>>) -> JobState {
        for _ in 0..3_000 {
            if let Some(state) = events.lock().unwrap().iter().find_map(|e| match e {
                TestEvent::Progress { state, .. } if *state != JobState::Running => Some(*state),
                _ => None,
            }) {
                return state;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("the bake job never reported a terminal state within 30 s");
    }

    fn samples_of(documents: &DocumentService) -> Vec<f32> {
        let source = documents.export_source().unwrap();
        let mut reader = SnapshotReader::new(source.store, source.snapshot);
        let mut out = vec![0.0f32; source.len_samples as usize];
        let mut pos = 0;
        while pos < out.len() {
            let n = reader.read(pos as u64, &mut out[pos..]).unwrap();
            assert!(n > 0);
            pos += n;
        }
        out
    }

    fn bits(v: &[f32]) -> Vec<u32> {
        v.iter().map(|s| s.to_bits()).collect()
    }

    fn voice() -> Vec<f32> {
        let mut x = vox_testkit::signal::pink_noise(3, -20.0, 2.0, 48_000).unwrap();
        x.extend(vox_testkit::signal::sine(700.0, -6.0, 0.5, 48_000).unwrap());
        x
    }

    /// The export render of `[start, end)` of the current document through `model` — what the
    /// bake must equal bit for bit.
    fn export_render(h: &Harness, model: &RackModel, start: u64, end: u64) -> Vec<f32> {
        let source = h.documents.export_source().unwrap();
        vox_engine::bake::render_document_range_to_vec(
            &builtins(),
            model,
            DocumentRange {
                store: &source.store,
                snapshot: &source.snapshot,
                sample_rate_hz: source.sample_rate_hz,
                range: vox_project::validate_range(start, end, source.len_samples).unwrap(),
            },
            &CancelToken::new(),
            |_| {},
        )
        .unwrap()
    }

    #[test]
    fn ac16_a_whole_file_bake_equals_export_resets_the_rack_and_undo_redo_restore_both() {
        let x = voice();
        let len = x.len() as u64;
        let h = harness("ac16", &x, builtins());
        load_rack(&h.handle, &ac16_rack());
        let rack_before = h.handle.rack_model().unwrap();
        assert!(
            rack_before.slots[0].state.get("blob").is_some(),
            "a state blob"
        );
        let expected = export_render(&h, &rack_before, 0, len);
        let original = samples_of(&h.documents);

        h.service.start_job(0, len).unwrap();
        assert_eq!(wait_for_finish(&h.events), JobState::Done);
        assert!(
            !h.documents.is_normalize_busy(),
            "the busy flag is released"
        );
        assert!(
            !h.documents.is_bake_running(),
            "H-30: the rack-edit guard is released once the bake commits"
        );

        // The baked audio is the export render (latency trimmed, tail included), bit for bit.
        let baked = samples_of(&h.documents);
        assert_eq!(baked.len(), x.len(), "a bake keeps the length");
        assert_eq!(bits(&baked), bits(&expected), "bake == export");
        // OD-4 default: the rack is reset; one `history.bake` entry.
        assert!(h.handle.rack_model().unwrap().slots.is_empty());
        let history = h.documents.history_state();
        assert!(history.can_undo);
        assert_eq!(history.undo_label.as_deref(), Some("history.bake"));
        let got = h.events.lock().unwrap().clone();
        assert!(got.contains(&TestEvent::DocumentChanged));
        assert!(
            got.iter()
                .any(|e| matches!(e, TestEvent::Notice(k, _) if k == "notice.bake.done"))
        );

        // Undo: the audio and the whole rack description equal their pre-bake values.
        h.documents.history_undo().unwrap();
        assert_eq!(bits(&samples_of(&h.documents)), bits(&original));
        assert_eq!(h.handle.rack_model().unwrap(), rack_before);
        // Redo: the baked audio and the reset rack.
        h.documents.history_redo().unwrap();
        assert_eq!(bits(&samples_of(&h.documents)), bits(&expected));
        assert!(h.handle.rack_model().unwrap().slots.is_empty());
        // And once more, to be sure the attachment survives a round trip.
        h.documents.history_undo().unwrap();
        assert_eq!(h.handle.rack_model().unwrap(), rack_before);
    }

    #[test]
    fn a_selection_bake_leaves_the_outside_bit_exact_and_equals_the_selection_export() {
        let x = voice();
        let (s, e) = (30_000u64, 90_000u64);
        let h = harness("selection", &x, builtins());
        load_rack(&h.handle, &ac16_rack());
        let rack = h.handle.rack_model().unwrap();
        let expected = export_render(&h, &rack, s, e);

        h.service.start_job(s, e).unwrap();
        assert_eq!(wait_for_finish(&h.events), JobState::Done);
        let baked = samples_of(&h.documents);
        assert_eq!(bits(&baked[..s as usize]), bits(&x[..s as usize]));
        assert_eq!(bits(&baked[e as usize..]), bits(&x[e as usize..]));
        assert_eq!(bits(&baked[s as usize..e as usize]), bits(&expected));
    }

    #[test]
    fn cancel_leaves_the_document_and_the_rack_unchanged() {
        // Long enough that the render is still running when the cancel arrives.
        let x = vox_testkit::signal::pink_noise(5, -20.0, 60.0, 48_000).unwrap();
        let len = x.len() as u64;
        let h = harness("cancel", &x, builtins());
        load_rack(&h.handle, &ac16_rack());
        let rack_before = h.handle.rack_model().unwrap();
        let rev = h.documents.info().audio_rev;

        let job = h.service.start_job(0, len).unwrap();
        h.service.cancel_job(job);
        assert_eq!(wait_for_finish(&h.events), JobState::Cancelled);
        assert!(!h.documents.is_normalize_busy());
        assert_eq!(h.documents.info().audio_rev, rev, "same revision");
        assert!(!h.documents.history_state().can_undo, "no undo entry");
        assert_eq!(
            h.handle.rack_model().unwrap(),
            rack_before,
            "the rack is kept"
        );
        assert!(
            !h.events
                .lock()
                .unwrap()
                .iter()
                .any(|e| matches!(e, TestEvent::Notice(..) | TestEvent::DocumentChanged)),
            "a cancellation posts no notice and changes nothing"
        );
    }

    /// A factory for `TestNaN`: a slot whose output turns non-finite — the in-process stand-in
    /// for a failing plugin (the sandboxed crash is `crates/sandbox/tests/bake.rs`).
    struct NanFactory(ModuleDescriptor);

    impl ModuleFactory for NanFactory {
        fn descriptor(&self) -> &ModuleDescriptor {
            &self.0
        }
        fn create(&self) -> Result<Box<dyn Module>, ModuleError> {
            Ok(Box::new(TestNaN::new(10_000)))
        }
    }

    #[test]
    fn a_failing_slot_aborts_the_bake_with_the_document_and_rack_unchanged() {
        let x = voice();
        let len = x.len() as u64;
        let mut factories = vox_modules::builtin_factories();
        factories.push(Arc::new(NanFactory(TestNaN::new(0).descriptor().clone())));
        let h = harness(
            "failing-slot",
            &x,
            Registry::with_factories(factories).unwrap(),
        );
        // The live engine doesn't know the module (a placeholder there, kept verbatim in the
        // model); the bake's registry does, and its render fails.
        let mut model = ac16_rack();
        model.slots.push(slot(TestNaN::ID, false, &[], None));
        load_rack(&h.handle, &model);
        let rack_before = h.handle.rack_model().unwrap();
        let rev = h.documents.info().audio_rev;

        h.service.start_job(0, len).unwrap();
        assert_eq!(wait_for_finish(&h.events), JobState::Failed);
        assert!(!h.documents.is_normalize_busy());
        assert!(
            !h.documents.is_bake_running(),
            "H-30: also released on failure"
        );
        assert_eq!(h.documents.info().audio_rev, rev);
        assert!(!h.documents.history_state().can_undo);
        assert_eq!(h.handle.rack_model().unwrap(), rack_before);
        let got = h.events.lock().unwrap().clone();
        assert!(
            got.iter().any(|e| matches!(
                e,
                TestEvent::Notice(k, p) if k == "error.bake.slot_failed"
                    && p.iter().any(|(n, v)| n == "slot" && v == "Test NaN")
            )),
            "{got:?}"
        );
    }

    #[test]
    fn busy_rules() {
        let x = vox_testkit::signal::pink_noise(6, -20.0, 60.0, 48_000).unwrap();
        let len = x.len() as u64;
        let h = harness("busy", &x, builtins());

        // An empty rack, or one whose slots are all bypassed, has nothing to bake.
        load_rack(&h.handle, &RackModel::default());
        let err = h.service.start_job(0, len).unwrap_err();
        assert_eq!(err.key, "error.bake.rack_inactive");
        let all_bypassed = RackModel {
            slots: vec![slot(Gain::ID, true, &[("gain_db", -6.0)], None)],
        };
        load_rack(&h.handle, &all_bypassed);
        let err = h.service.start_job(0, len).unwrap_err();
        assert_eq!(err.key, "error.bake.rack_inactive");
        assert!(
            !h.documents.is_normalize_busy(),
            "refused before the busy flag is set"
        );
        // An empty range is refused like every selection-scoped op.
        load_rack(&h.handle, &ac16_rack());
        assert_eq!(
            h.service.start_job(10, 10).unwrap_err().key,
            "error.no_selection"
        );

        // While a bake runs: a second bake, edits, undo/redo and normalize are refused, and
        // (H-30) `is_bake_running` is set so `ipc::rack_commands` refuses rack edits too.
        let job = h.service.start_job(0, len).unwrap();
        assert!(h.documents.is_bake_running());
        assert_eq!(
            h.service.start_job(0, len).unwrap_err().code,
            IpcErrorCode::Busy
        );
        assert_eq!(
            h.documents.edit_delete(0, 100).unwrap_err().code,
            IpcErrorCode::Busy
        );
        assert_eq!(
            h.documents.history_undo().unwrap_err().code,
            IpcErrorCode::Busy
        );
        assert_eq!(
            h.documents.begin_normalize_job(0, len).err().unwrap().code,
            IpcErrorCode::Busy
        );
        h.service.cancel_job(job);
        assert_eq!(wait_for_finish(&h.events), JobState::Cancelled);
        assert!(!h.documents.is_normalize_busy());
        assert!(
            !h.documents.is_bake_running(),
            "H-30: a cancelled bake also releases the rack-edit guard"
        );
    }

    #[test]
    fn no_document_is_refused() {
        let (_engine, handle, _driver) = live_engine();
        let dir = tmp_dir("no-doc");
        load_rack(&handle, &ac16_rack());
        let documents = DocumentService::new(dir.join("sessions"), handle.clone());
        let service = BakeService(Arc::new(Inner {
            documents,
            engine: handle,
            registry: Arc::new(builtins()),
            spectro: None,
            emit: Arc::new(|_| {}),
            next_id: AtomicU32::new(1),
            jobs: Mutex::new(HashMap::new()),
        }));
        assert_eq!(
            service.start_job(0, 10).unwrap_err().key,
            "error.document.none"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rack_activity() {
        assert!(!rack_is_active(&RackModel::default()));
        assert!(rack_is_active(&ac16_rack()));
        let bypassed = RackModel {
            slots: vec![slot(Gain::ID, true, &[], None)],
        };
        assert!(!rack_is_active(&bypassed));
    }
}

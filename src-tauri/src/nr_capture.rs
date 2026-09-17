//! Capture Noise Print job (S3-06, SPEC-014 §2.3): mirrors `export.rs`'s job shape.
//!
//! `nr_capture_start` resolves (or inserts) the target Noise Reduction slot synchronously
//! (`EngineHandle::nr_capture_prepare`, fast: no audio read yet) and returns its index
//! immediately, then runs the rest on its own thread:
//! 1. read `[start − preroll, analysed_end)` of the document at its own rate (the excerpt is
//!    capped to the first 60 s of the *selection*, SPEC-014 §2.3);
//! 2. render it through the target slot's upstream model (`vox_rack::offline::render`) — skipped
//!    entirely when nothing precedes the slot, per spec — then drop the pre-roll;
//! 3. `NoiseProfile::capture` the result;
//! 4. install the blob live (`EngineHandle::nr_capture_apply`).
//!
//! Never touches the document (D-019). Checks that block the button (no selection, recording)
//! are re-checked here as a safe no-op; SPEC-014 §2.3's length/silence/loudness/60 s-cap checks
//! are host-side (the module has no API for them) and run here, against the excerpt this job
//! itself reads.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use tauri::{AppHandle, Runtime};
use vox_engine::{EngineHandle, NrCapturePrep, RackApiError};
use vox_project::{CHUNK_SAMPLES, SnapshotReader};
use vox_rack::Registry;

use crate::document::{DocumentService, ExportSource};
use crate::ipc::{
    IpcError, IpcErrorCode, JobKind, JobProgressDto, JobState, Notice, NoticeLevel,
    emit_job_progress, emit_notice,
};

/// SPEC-014 §3.3 `capture_preroll_s`: at most this much pre-roll, clipped to `start`.
const PREROLL_S: f64 = 1.0;
/// SPEC-014 §3.3 `short_capture_warn_s`: below this, capture still succeeds but warns.
const SHORT_WARN_S: f64 = 1.0;
/// SPEC-014 §2.3: above this RMS, capture still succeeds but warns the selection may hold speech.
const LOUD_WARN_DBFS: f64 = -35.0;

/// `NrCaptureService::start_job`'s argument: the selection (`[start, end)`, document samples) and
/// the last-focused NR slot, if any (SPEC-014 §2.3 "target slot").
pub struct NrCaptureRequest {
    pub hint_slot: Option<usize>,
    pub start: u64,
    pub end: u64,
}

enum NrCaptureEvent {
    Progress(JobProgressDto),
    Notice(Notice),
}

type NrCaptureEmitter = Arc<dyn Fn(NrCaptureEvent) + Send + Sync>;

struct JobHandle {
    cancel: Arc<AtomicBool>,
}

struct Inner {
    documents: DocumentService,
    engine: EngineHandle,
    registry: Arc<Registry>,
    emit: NrCaptureEmitter,
    next_id: AtomicU32,
    jobs: Mutex<std::collections::HashMap<u32, JobHandle>>,
}

/// Cheaply cloneable (an `Arc` inside), managed as Tauri state like `ExportService`.
#[derive(Clone)]
pub struct NrCaptureService(Arc<Inner>);

/// Builds the service (its own `Registry` instance, like `ExportService` — modules are stateless
/// per-instance, so this only costs one small allocation, and it must resolve the same upstream
/// module ids as the live rack regardless).
pub fn start<R: Runtime>(
    app: AppHandle<R>,
    engine: EngineHandle,
    documents: DocumentService,
) -> anyhow::Result<NrCaptureService> {
    let registry = crate::plugins::registry()?;
    let emit: NrCaptureEmitter = Arc::new(move |event| forward(&app, event));
    Ok(NrCaptureService(Arc::new(Inner {
        documents,
        engine,
        registry,
        emit,
        next_id: AtomicU32::new(1),
        jobs: Mutex::new(std::collections::HashMap::new()),
    })))
}

fn forward<R: Runtime>(app: &AppHandle<R>, event: NrCaptureEvent) {
    let result = match event {
        NrCaptureEvent::Progress(dto) => emit_job_progress(app, dto),
        NrCaptureEvent::Notice(notice) => emit_notice(app, notice),
    };
    if let Err(error) = result {
        tracing::warn!(%error, "emitting an nr-capture event failed");
    }
}

fn no_selection() -> IpcError {
    IpcError::new(IpcErrorCode::InvalidArgument, "error.no_selection")
}

fn invalid_range() -> IpcError {
    IpcError::new(IpcErrorCode::InvalidArgument, "error.invalid_range")
}

fn cancelled() -> IpcError {
    IpcError::new(IpcErrorCode::Cancelled, "error.cancelled")
}

fn check_cancelled(cancel: &AtomicBool) -> Result<(), IpcError> {
    if cancel.load(Ordering::Relaxed) {
        Err(cancelled())
    } else {
        Ok(())
    }
}

/// The capture path's rack failures map exactly like every other rack command's (H-77: one
/// match, so a new `RackApiError` variant can't drift between the two).
fn nr_rack_error(err: RackApiError) -> IpcError {
    crate::ipc::rack_ipc_error(err)
}

fn project_read_error(err: vox_project::ProjectError) -> IpcError {
    IpcError::new(IpcErrorCode::Io, "error.nr_capture.read").with_param("message", err.to_string())
}

fn render_error(err: vox_rack::offline::RenderError) -> IpcError {
    IpcError::new(IpcErrorCode::Internal, "error.nr_capture.render")
        .with_param("message", err.to_string())
}

fn capture_error(err: vox_rack::ModuleError) -> IpcError {
    IpcError::new(IpcErrorCode::Internal, "error.nr_capture.capture_failed")
        .with_param("message", err.to_string())
}

fn notice_from_error(err: &IpcError) -> Notice {
    let mut notice = Notice::toast(NoticeLevel::Error, err.key.clone());
    for (name, value) in &err.params {
        notice = notice.with_param(name.clone(), value.clone());
    }
    notice
}

impl NrCaptureService {
    /// `nr_capture_start`: resolves the target slot (may insert one, reported live through the
    /// engine's own `rack_changed`) and validates the selection, then runs the rest on its own
    /// thread. Returns the job id and the resolved slot index immediately.
    pub fn start_job(&self, request: NrCaptureRequest) -> Result<(u32, usize), IpcError> {
        if self.0.documents.is_recording() {
            return Err(IpcError::not_while_recording());
        }
        let source = self.0.documents.export_source()?;
        if request.start >= request.end {
            return Err(no_selection());
        }
        if request.end > source.len_samples {
            return Err(invalid_range());
        }
        let prep = self
            .0
            .engine
            .nr_capture_prepare(request.hint_slot)
            .map_err(nr_rack_error)?;
        if prep.inserted {
            (self.0.emit)(NrCaptureEvent::Notice(Notice::toast(
                NoticeLevel::Info,
                "notice.nr_capture.slot_added",
            )));
        }

        let job_id = self.0.next_id.fetch_add(1, Ordering::Relaxed);
        let cancel = Arc::new(AtomicBool::new(false));
        self.0.jobs.lock().unwrap().insert(
            job_id,
            JobHandle {
                cancel: Arc::clone(&cancel),
            },
        );
        let inner = Arc::clone(&self.0);
        let slot = prep.index;
        std::thread::Builder::new()
            .name("nr-capture-job".into())
            .spawn(move || {
                run_job(
                    inner,
                    job_id,
                    source,
                    prep,
                    request.start,
                    request.end,
                    cancel,
                )
            })
            .map_err(|e| IpcError::internal(e.to_string()))?;
        Ok((job_id, slot))
    }

    /// `nr_capture_cancel`: best-effort — the job notices at the next `cancel` poll (offline
    /// render blocks, or every 64 analysis frames inside `capture`) and leaves the previous print,
    /// if any, unchanged. A no-op for an unknown or already-finished job id.
    pub fn cancel_job(&self, job_id: u32) {
        if let Some(job) = self.0.jobs.lock().unwrap().get(&job_id) {
            job.cancel.store(true, Ordering::Relaxed);
        }
    }
}

fn run_job(
    inner: Arc<Inner>,
    job_id: u32,
    source: ExportSource,
    prep: NrCapturePrep,
    start: u64,
    end: u64,
    cancel: Arc<AtomicBool>,
) {
    let emit = Arc::clone(&inner.emit);
    let result = run_pipeline(&inner, &source, &prep, start, end, &cancel, |fraction| {
        emit(NrCaptureEvent::Progress(JobProgressDto {
            job_id,
            kind: JobKind::NrCapture,
            state: JobState::Running,
            fraction,
        }));
    });
    inner.jobs.lock().unwrap().remove(&job_id);

    let (state, fraction) = match &result {
        Ok(_) => (JobState::Done, 1.0),
        Err(err) if err.code == IpcErrorCode::Cancelled => (JobState::Cancelled, 0.0),
        Err(_) => (JobState::Failed, 0.0),
    };
    // H-30: the error notice goes out before the terminal `Failed` progress event (bake.rs's
    // `fail_job` convention), so anything that sees `Failed` already has the reason. Cancelled
    // still posts nothing (the previous print, if any, is untouched) and a success's warnings
    // still follow the `Done` progress event — neither of those orderings is load-bearing.
    if let (JobState::Failed, Err(err)) = (state, &result) {
        (inner.emit)(NrCaptureEvent::Notice(notice_from_error(err)));
    }
    (inner.emit)(NrCaptureEvent::Progress(JobProgressDto {
        job_id,
        kind: JobKind::NrCapture,
        state,
        fraction,
    }));
    if let Ok(warnings) = result {
        for key in warnings {
            (inner.emit)(NrCaptureEvent::Notice(Notice::toast(
                NoticeLevel::Warning,
                key,
            )));
        }
    }
}

/// Reads `[start, end)` of `source`'s current snapshot into memory (same approach as `export.rs`'s
/// `read_range`, duplicated locally to keep this ticket's edits out of that file).
fn read_range(source: &ExportSource, start: u64, end: u64) -> Result<Vec<f32>, IpcError> {
    let mut reader = SnapshotReader::new(Arc::clone(&source.store), Arc::clone(&source.snapshot));
    let mut samples = Vec::with_capacity((end - start) as usize);
    let mut buf = vec![0.0f32; CHUNK_SAMPLES];
    let mut pos = start;
    while pos < end {
        let want = ((end - pos) as usize).min(buf.len());
        let n = reader
            .read(pos, &mut buf[..want])
            .map_err(project_read_error)?;
        if n == 0 {
            break;
        }
        samples.extend_from_slice(&buf[..n]);
        pos += n as u64;
    }
    Ok(samples)
}

/// Read → (render through the upstream model) → capture → apply. Returns the i18n keys of every
/// warning to post on success (SPEC-014 §2.3: short/loud/60 s-cap).
#[allow(clippy::too_many_arguments)]
fn run_pipeline(
    inner: &Inner,
    source: &ExportSource,
    prep: &NrCapturePrep,
    start: u64,
    end: u64,
    cancel: &AtomicBool,
    mut progress: impl FnMut(f32),
) -> Result<Vec<&'static str>, IpcError> {
    check_cancelled(cancel)?;
    let fs = f64::from(source.sample_rate_hz);
    let sel_len = end - start;

    let min_samples = prep.extension.min_capture_samples(fs, &prep.values);
    if (sel_len as usize) < min_samples {
        return Err(IpcError::new(
            IpcErrorCode::InvalidArgument,
            "error.nr_capture.too_short",
        ));
    }
    let mut warnings = Vec::new();
    if (sel_len as f64) < (SHORT_WARN_S * fs).round() {
        warnings.push("notice.nr_capture.short");
    }
    let max_samples = vox_dsp::nr::max_capture_samples(fs) as u64;
    let capped = max_samples > 0 && sel_len > max_samples;
    let analysed_end = if capped { start + max_samples } else { end };
    if capped {
        warnings.push("notice.nr_capture.capped_60s");
    }

    let upstream_empty = prep.upstream_model.slots.is_empty();
    let preroll = if upstream_empty {
        0
    } else {
        ((PREROLL_S * fs).round() as u64).min(start)
    };

    progress(0.05);
    check_cancelled(cancel)?;
    let read = read_range(source, start - preroll, analysed_end)?;

    progress(0.3);
    check_cancelled(cancel)?;
    let excerpt: Vec<f32> = if upstream_empty {
        read
    } else {
        let rendered = vox_rack::offline::render(&inner.registry, &prep.upstream_model, fs, &read)
            .map_err(render_error)?;
        rendered[preroll as usize..].to_vec()
    };

    progress(0.6);
    if excerpt.iter().all(|&s| s == 0.0) {
        return Err(IpcError::new(
            IpcErrorCode::InvalidArgument,
            "error.nr_capture.silent",
        ));
    }
    let ms = excerpt
        .iter()
        .map(|&s| f64::from(s) * f64::from(s))
        .sum::<f64>()
        / excerpt.len() as f64;
    if ms.sqrt().log10() * 20.0 > LOUD_WARN_DBFS {
        warnings.push("notice.nr_capture.loud");
    }

    check_cancelled(cancel)?;
    let blob = prep
        .extension
        .capture(&excerpt, fs, &prep.values, cancel)
        .map_err(|e| {
            if cancel.load(Ordering::Relaxed) {
                cancelled()
            } else {
                capture_error(e)
            }
        })?;

    progress(0.9);
    check_cancelled(cancel)?;
    inner
        .engine
        .nr_capture_apply(prep.index, blob)
        .map_err(nr_rack_error)?;
    progress(1.0);
    Ok(warnings)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use vox_engine::backend::fake::{FakeBackend, FakeDevice, FakeDirection};
    use vox_engine::{Engine, EngineConfig, HostId, RackCommand};
    use vox_project::{ChunkStore, DocSnapshot, StoreOptions};
    use vox_rack::Registry;

    use super::*;

    fn tmp_dir(tag: &str) -> std::path::PathBuf {
        crate::test_util::tmp_dir(&format!("nr-capture-{tag}"))
    }

    /// White noise in `[-amp, amp)`, seeded (xorshift64*, no extra test dependency).
    fn white_noise(seed: u64, amp: f32, n: usize) -> Vec<f32> {
        let mut s = seed | 1;
        (0..n)
            .map(|_| {
                s ^= s >> 12;
                s ^= s << 25;
                s ^= s >> 27;
                let u = (s.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 11) as f64 / (1u64 << 53) as f64;
                amp * (2.0 * u - 1.0) as f32
            })
            .collect()
    }

    fn source_from(dir: &std::path::Path, samples: &[f32], rate_hz: u32) -> ExportSource {
        let store =
            ChunkStore::create(dir, 0, StoreOptions::with_memory_budget(64 * 1024 * 1024)).unwrap();
        let mut writer = store.writer();
        writer.append(samples).unwrap();
        let audio = writer.finish().unwrap();
        let snapshot = Arc::new(DocSnapshot::new(rate_hz, audio.pieces, Vec::new()));
        ExportSource {
            store,
            snapshot,
            sample_rate_hz: rate_hz,
            len_samples: samples.len() as u64,
            suggested_name: None,
        }
    }

    fn registry() -> Registry {
        Registry::with_factories(vox_modules::builtin_factories()).unwrap()
    }

    fn doc_service_for(dir: &std::path::Path, engine: EngineHandle) -> super::DocumentService {
        super::DocumentService::new(dir.join("sessions"), engine)
    }

    /// A stopped-rack engine (no device plugged): enough for the error paths that never reach
    /// `EngineHandle::nr_capture_apply` (a synthetic [`NrCapturePrep`] with no real slot covers
    /// them without a real, live rack).
    fn stopped_engine() -> (Engine, EngineHandle) {
        let fake = FakeBackend::new(1);
        let registry =
            Arc::new(Registry::with_factories(vox_modules::builtin_factories()).unwrap());
        let engine = Engine::start(EngineConfig::new(Arc::new(fake), registry)).unwrap();
        let handle = engine.handle();
        (engine, handle)
    }

    /// A synthetic prep with no upstream slots, for the error paths above: `min_capture_samples`
    /// is configurable, `capture` runs the real `dsp` analysis (so silence/short/loud math is
    /// exercised for real), and `describe` is unused.
    fn synthetic_prep(min_capture_s: f64) -> NrCapturePrep {
        struct Ext(f64);
        impl vox_rack::NoiseProfile for Ext {
            fn capture(
                &self,
                noise: &[f32],
                sample_rate: f64,
                _values: &[f64],
                cancel: &AtomicBool,
            ) -> Result<Vec<u8>, vox_rack::ModuleError> {
                vox_dsp::nr::capture_profile(noise, sample_rate, cancel)
                    .map_err(|e| vox_rack::ModuleError::Resource(e.to_string()))
            }
            fn min_capture_samples(&self, sample_rate: f64, _values: &[f64]) -> usize {
                (self.0 * sample_rate).round() as usize
            }
            fn describe(
                &self,
                _blob: &[u8],
                _out: &mut Vec<(f32, f32)>,
            ) -> Result<(), vox_rack::ModuleError> {
                Ok(())
            }
        }
        NrCapturePrep {
            index: 0,
            inserted: false,
            upstream_model: vox_rack::RackModel { slots: Vec::new() },
            values: Vec::new(),
            extension: Arc::new(Ext(min_capture_s)),
        }
    }

    /// A live engine (a fake output device opens for real) with one real Noise Reduction slot at
    /// index 0 — for the tests that exercise the whole pipeline through `nr_capture_apply`, which
    /// needs an actual slot to replace. Keeps its own `FakeDriver` alive: without one, the fake
    /// stream's callback never runs, and the stall detector closes the "live" output out from
    /// under a slow test (debug-build FFTs make `capture()` slow enough to cross the 1 s device
    /// poll otherwise) — mirrors `recording.rs`'s threaded-engine test.
    fn live_engine_with_nr_slot() -> (Engine, EngineHandle, vox_engine::backend::fake::FakeDriver) {
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
        let mut opened = false;
        for _ in 0..500 {
            if handle
                .rack_command(RackCommand::Add {
                    module_id: vox_modules::NoiseReduction::ID.into(),
                    index: 0,
                })
                .is_ok()
            {
                opened = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(opened, "the fake output device never opened a live rack");
        (engine, handle, driver)
    }

    /// `nr_capture_prepare`, retried: a busy test machine can stall a test thread long enough for
    /// the fake backend's device poll (1 s) to momentarily cycle the output stream between
    /// `live_engine_with_nr_slot`'s own retry loop succeeding and this call.
    fn retry_prepare(handle: &EngineHandle, hint: Option<usize>) -> NrCapturePrep {
        for _ in 0..500 {
            if let Ok(prep) = handle.nr_capture_prepare(hint) {
                return prep;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!("nr_capture_prepare never succeeded on the live rack");
    }

    /// `run_pipeline`, retried against the same transient device-poll window as
    /// [`retry_prepare`] — the pipeline's own `capture()` step (a real FFT analysis) can take
    /// long enough on a busy test machine for the fake output to have cycled by the time it
    /// reaches `nr_capture_apply`.
    #[allow(clippy::too_many_arguments)]
    fn retry_run_pipeline(
        inner: &Inner,
        source: &ExportSource,
        prep: &NrCapturePrep,
        start: u64,
        end: u64,
    ) -> Result<Vec<&'static str>, IpcError> {
        for attempt in 0..10 {
            match run_pipeline(
                inner,
                source,
                prep,
                start,
                end,
                &AtomicBool::new(false),
                |_| {},
            ) {
                Err(err) if err.key == "error.rack_unavailable" && attempt < 9 => {
                    std::thread::sleep(Duration::from_millis(50));
                }
                other => return other,
            }
        }
        unreachable!()
    }

    #[test]
    fn too_short_selection_is_rejected_before_reading_any_more_than_the_selection() {
        let dir = tmp_dir("too-short");
        let (_engine, handle) = stopped_engine();
        let documents = doc_service_for(&dir, handle.clone());
        let inner = Inner {
            documents,
            engine: handle,
            registry: Arc::new(registry()),
            emit: Arc::new(|_| {}),
            next_id: AtomicU32::new(1),
            jobs: Mutex::new(std::collections::HashMap::new()),
        };
        let source = source_from(&dir, &white_noise(1, 0.01, 24_000), 48_000);
        let prep = synthetic_prep(0.5); // needs 24_000 samples at 48 kHz

        let err = run_pipeline(
            &inner,
            &source,
            &prep,
            0,
            23_999,
            &AtomicBool::new(false),
            |_| {},
        )
        .unwrap_err();
        assert_eq!(err.key, "error.nr_capture.too_short");
    }

    #[test]
    fn a_digitally_silent_selection_is_rejected() {
        let dir = tmp_dir("silent");
        let (_engine, handle) = stopped_engine();
        let documents = doc_service_for(&dir, handle.clone());
        let inner = Inner {
            documents,
            engine: handle,
            registry: Arc::new(registry()),
            emit: Arc::new(|_| {}),
            next_id: AtomicU32::new(1),
            jobs: Mutex::new(std::collections::HashMap::new()),
        };
        let source = source_from(&dir, &vec![0.0f32; 48_000], 48_000);
        let prep = synthetic_prep(0.5);

        let err = run_pipeline(
            &inner,
            &source,
            &prep,
            0,
            48_000,
            &AtomicBool::new(false),
            |_| {},
        )
        .unwrap_err();
        assert_eq!(err.key, "error.nr_capture.silent");
    }

    #[test]
    fn cancelling_before_the_job_starts_reports_cancelled() {
        let dir = tmp_dir("cancel");
        let (_engine, handle) = stopped_engine();
        let documents = doc_service_for(&dir, handle.clone());
        let inner = Inner {
            documents,
            engine: handle,
            registry: Arc::new(registry()),
            emit: Arc::new(|_| {}),
            next_id: AtomicU32::new(1),
            jobs: Mutex::new(std::collections::HashMap::new()),
        };
        let samples = white_noise(5, 0.001, 48_000);
        let source = source_from(&dir, &samples, 48_000);
        let prep = synthetic_prep(0.5);

        let err = run_pipeline(
            &inner,
            &source,
            &prep,
            0,
            samples.len() as u64,
            &AtomicBool::new(true),
            |_| {},
        )
        .unwrap_err();
        assert_eq!(err.code, IpcErrorCode::Cancelled);
    }

    #[test]
    fn start_job_refuses_no_open_document() {
        let dir = tmp_dir("start-job-guards");
        let (_engine, handle) = stopped_engine();
        let documents = doc_service_for(&dir, handle.clone());
        // No document open: `export_source` refuses first (mirrors export's own guard).
        let inner = Arc::new(Inner {
            documents,
            engine: handle,
            registry: Arc::new(registry()),
            emit: Arc::new(|_| {}),
            next_id: AtomicU32::new(1),
            jobs: Mutex::new(std::collections::HashMap::new()),
        });
        let service = NrCaptureService(inner);
        let err = service
            .start_job(NrCaptureRequest {
                hint_slot: None,
                start: 0,
                end: 100,
            })
            .unwrap_err();
        assert_eq!(err.key, "error.document.none");
    }

    /// End to end, through a real live rack: capturing installs the print (SPEC-014 §2.3
    /// "Result"), and each SPEC-014 §2.3 warning (short / loud / 60 s cap) is reported without
    /// failing the job.
    #[test]
    fn full_pipeline_installs_the_print_and_reports_warnings() {
        let dir = tmp_dir("full-pipeline");
        let (_engine, handle, _driver) = live_engine_with_nr_slot();
        let documents = doc_service_for(&dir, handle.clone());
        let inner = Inner {
            documents,
            engine: handle.clone(),
            registry: Arc::new(registry()),
            emit: Arc::new(|_| {}),
            next_id: AtomicU32::new(1),
            jobs: Mutex::new(std::collections::HashMap::new()),
        };

        // -20 dBFS RMS, 0.6 s (>= the 0.5 s minimum, short of the 1.0 s warning threshold, and
        // loud enough to also trip the room-tone warning): both fire together.
        let samples = white_noise(2, 0.1, (0.6 * 48_000.0) as usize);
        let source = source_from(&dir, &samples, 48_000);
        let prep = retry_prepare(&handle, None);
        assert_eq!(prep.index, 0);
        assert!(!prep.inserted, "the slot was already there");

        let warnings = retry_run_pipeline(&inner, &source, &prep, 0, samples.len() as u64).unwrap();
        assert!(warnings.contains(&"notice.nr_capture.short"));
        assert!(warnings.contains(&"notice.nr_capture.loud"));

        let snap = handle.rack_snapshot();
        assert_eq!(
            snap.slots[0].info.noise_profile,
            Some(vox_rack::NoiseProfileStatus::Loaded),
            "the captured print is now the slot's committed blob"
        );
    }

    #[test]
    fn a_selection_over_60_s_is_capped_with_a_notice() {
        let dir = tmp_dir("cap-60s");
        let (_engine, handle, _driver) = live_engine_with_nr_slot();
        let documents = doc_service_for(&dir, handle.clone());
        let inner = Inner {
            documents,
            engine: handle.clone(),
            registry: Arc::new(registry()),
            emit: Arc::new(|_| {}),
            next_id: AtomicU32::new(1),
            jobs: Mutex::new(std::collections::HashMap::new()),
        };
        let samples = white_noise(4, 0.001, 61 * 48_000);
        let source = source_from(&dir, &samples, 48_000);
        let prep = retry_prepare(&handle, None);

        let warnings = retry_run_pipeline(&inner, &source, &prep, 0, samples.len() as u64).unwrap();
        assert!(warnings.contains(&"notice.nr_capture.capped_60s"));
    }

    #[derive(Debug, Clone, PartialEq)]
    enum TestEvent {
        Progress(JobState),
        Notice(String),
    }

    fn wait_for_finish(events: &Mutex<Vec<TestEvent>>) -> JobState {
        for _ in 0..500 {
            if let Some(state) = events.lock().unwrap().iter().find_map(|e| match e {
                TestEvent::Progress(state) if *state != JobState::Running => Some(*state),
                _ => None,
            }) {
                return state;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("the nr_capture job never reported a terminal state within 5s");
    }

    /// H-30: a failed job's error notice is already posted by the time `wait_for_finish` sees the
    /// terminal `Failed` state — no extra polling for the notice itself (the point of the
    /// ordering fix). A selection shorter than `min_capture_samples` deterministically fails
    /// inside `run_pipeline`, before any audio is even read — no timing race needed.
    #[test]
    fn a_failed_job_posts_the_notice_before_the_terminal_failed_progress_event() {
        let dir = tmp_dir("failed-order");
        let (_engine, handle, _driver) = live_engine_with_nr_slot();
        let documents = doc_service_for(&dir, handle.clone());
        let samples = white_noise(9, 0.1, 48_000);
        let path = dir.join("in.wav");
        vox_testkit::wav::write_wav_file(
            &path,
            &samples,
            1,
            48_000,
            vox_testkit::wav::BitDepth::Float32,
        )
        .unwrap();
        documents.open(&path, false).unwrap();

        let events: Arc<Mutex<Vec<TestEvent>>> = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&events);
        let emit: NrCaptureEmitter = Arc::new(move |event| {
            let e = match event {
                NrCaptureEvent::Progress(dto) => TestEvent::Progress(dto.state),
                NrCaptureEvent::Notice(n) => TestEvent::Notice(n.key),
            };
            sink.lock().unwrap().push(e);
        });
        let inner = Arc::new(Inner {
            documents,
            engine: handle,
            registry: Arc::new(registry()),
            emit,
            next_id: AtomicU32::new(1),
            jobs: Mutex::new(std::collections::HashMap::new()),
        });
        let service = NrCaptureService(inner);

        // Far short of the NoiseReduction extension's minimum (0.5 s / 24000 samples at 48 kHz).
        service
            .start_job(NrCaptureRequest {
                hint_slot: None,
                start: 0,
                end: 5_000,
            })
            .unwrap();
        assert_eq!(wait_for_finish(&events), JobState::Failed);

        let got = events.lock().unwrap().clone();
        assert!(
            got.iter().any(
                |e| matches!(e, TestEvent::Notice(key) if key == "error.nr_capture.too_short")
            ),
            "the notice must already be present once `Failed` is observed: {got:?}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// H-50: audited alongside normalize/export/loudness/bake's own success-path ordering bug —
    /// here the real "result" (the installed print) is already committed by
    /// `EngineHandle::nr_capture_apply` inside `run_pipeline`, synchronously before it returns
    /// `Ok`, so it is already installed by the time `run_job` even decides the terminal state is
    /// `Done` (unlike the other services, no reordering was needed). This locks that invariant in
    /// end to end through the real job thread, polled with `wait_for_finish` rather than raced.
    #[test]
    fn a_successful_job_installs_the_print_before_the_terminal_done_progress_event() {
        let dir = tmp_dir("success-order");
        let (_engine, handle, _driver) = live_engine_with_nr_slot();
        let documents = doc_service_for(&dir, handle.clone());
        // 2 s at a moderate level: comfortably past the short/loud warning thresholds, so the
        // job succeeds with no warnings to keep this test focused on the print alone.
        let samples = white_noise(11, 0.03, 2 * 48_000);
        let path = dir.join("in.wav");
        vox_testkit::wav::write_wav_file(
            &path,
            &samples,
            1,
            48_000,
            vox_testkit::wav::BitDepth::Float32,
        )
        .unwrap();
        documents.open(&path, false).unwrap();

        let events: Arc<Mutex<Vec<TestEvent>>> = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&events);
        let emit: NrCaptureEmitter = Arc::new(move |event| {
            let e = match event {
                NrCaptureEvent::Progress(dto) => TestEvent::Progress(dto.state),
                NrCaptureEvent::Notice(n) => TestEvent::Notice(n.key),
            };
            sink.lock().unwrap().push(e);
        });
        let inner = Arc::new(Inner {
            documents,
            engine: handle.clone(),
            registry: Arc::new(registry()),
            emit,
            next_id: AtomicU32::new(1),
            jobs: Mutex::new(std::collections::HashMap::new()),
        });
        let service = NrCaptureService(inner);

        let (_job_id, slot) = service
            .start_job(NrCaptureRequest {
                hint_slot: None,
                start: 0,
                end: samples.len() as u64,
            })
            .unwrap();
        assert_eq!(wait_for_finish(&events), JobState::Done);

        let snap = handle.rack_snapshot();
        assert_eq!(
            snap.slots[slot].info.noise_profile,
            Some(vox_rack::NoiseProfileStatus::Loaded),
            "the captured print must already be installed once Done is observed"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}

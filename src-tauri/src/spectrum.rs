//! Long-term average spectrum job (H-42, SPEC-007 §8.5): the analyzer's "Average" mode and the
//! Spectrum Inspector's document source. Reads a document range (the selection, or the whole
//! file), optionally renders it through the live rack (same offline chain as loudness/export,
//! `crate::loudness::render_processed_to_buffer`), and runs
//! `vox_dsp::diagnostics::analyze_buffer` on each requested signal — "processed", "source", or
//! both for the Source-vs-Processed comparison — on its own thread, off the audio thread.
//!
//! Reports progress as `job_progress` (`JobKind::SpectrumAnalyze`, ≤ 10 Hz), then a
//! `spectrum_report` event (JSON: the diagnostics per signal). The curves themselves are bulk
//! data, fetched afterwards as binary `VXLT` frames with `spectrum_analyze_curve(job_id, index)`
//! (CLAUDE.md: never JSON float arrays). The last [`KEPT_RESULTS`] jobs' curves are kept.
//!
//! Error notices go out before the terminal `Failed` progress event (H-30 convention); a
//! cancellation posts none.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tauri::{AppHandle, Runtime};
use vox_dsp::diagnostics::spectrum::{is_valid_fft_size, power_db};
use vox_dsp::diagnostics::{OfflineAnalysis, OfflineConfig, WindowKind, analyze_buffer};
use vox_engine::EngineHandle;
use vox_rack::{RackModel, Registry};

use crate::document::DocumentService;
use crate::ipc::VoiceReportDto;
use crate::ipc::spectrum_dto::{SpectrumReportDto, SpectrumResultDto};
use crate::ipc::{
    IpcError, IpcErrorCode, JobKind, JobProgressDto, JobState, Notice, NoticeLevel,
    emit_job_progress, emit_notice, emit_spectrum_report,
};
use crate::job_status::JobStatusRegistry;
use crate::loudness::{LoudnessSource, read_range, render_processed_to_buffer};

/// How many finished jobs keep their curves for `spectrum_analyze_curve`.
pub const KEPT_RESULTS: usize = 4;
/// `job_progress` events at most this often.
const PROGRESS_INTERVAL: Duration = Duration::from_millis(100);
/// `spectrum_export_csv` refuses anything larger (bytes).
pub const MAX_CSV_BYTES: usize = 64 << 20;

/// `VXLT` v1 header length (SPEC-007 §8.9).
pub const VXLT_HEADER_LEN: usize = 36;
/// `VXLT` flag: a room-tone (quiet-frame) curve follows the long-term average.
pub const VXLT_HAS_NOISE: u32 = 1 << 0;

/// `SpectrumService::start_job`'s argument.
pub struct SpectrumRequest {
    /// `[start, end)` samples; `None` = the whole document.
    pub range: Option<(u64, u64)>,
    /// Which signals to analyse, in order (at least one).
    pub sources: Vec<LoudnessSource>,
    pub fft_size: u32,
    pub window: WindowKind,
}

/// One analysed signal of a finished job.
pub struct CurveResult {
    pub source: LoudnessSource,
    pub analysis: OfflineAnalysis,
}

struct JobOutcome {
    job_id: u32,
    sample_rate_hz: u32,
    results: Vec<CurveResult>,
}

enum SpectrumEvent {
    Progress(JobProgressDto),
    Report(SpectrumReportDto),
    Notice(Notice),
}

type SpectrumEmitter = Arc<dyn Fn(SpectrumEvent) + Send + Sync>;

struct Inner {
    documents: DocumentService,
    engine: EngineHandle,
    registry: Arc<Registry>,
    emit: SpectrumEmitter,
    next_id: AtomicU32,
    jobs: Mutex<HashMap<u32, Arc<AtomicBool>>>,
    results: Mutex<VecDeque<Arc<JobOutcome>>>,
}

/// Cheaply cloneable (an `Arc` inside), like [`crate::loudness::LoudnessService`].
#[derive(Clone)]
pub struct SpectrumService(Arc<Inner>);

/// Builds the service (composition root; its own `Registry`, like the loudness service).
///
/// `status` is the app-wide [`JobStatusRegistry`] (H-96 "belt and braces") shared with
/// export/normalize/bake: every `job_progress` tick this service emits is also recorded there, so
/// a UI that ever misses the real event (H-92: "Explain My Voice" starting this job and needing to
/// recover rather than get stuck on "analysing…") can poll `job_status(job_id)` instead.
pub fn start<R: Runtime>(
    app: AppHandle<R>,
    documents: DocumentService,
    engine: EngineHandle,
    status: JobStatusRegistry,
) -> anyhow::Result<SpectrumService> {
    let registry = crate::plugins::registry()?;
    let emit: SpectrumEmitter = Arc::new(move |event| {
        if let SpectrumEvent::Progress(dto) = &event {
            status.record(*dto);
        }
        forward(&app, event);
    });
    Ok(SpectrumService(Arc::new(Inner {
        documents,
        engine,
        registry,
        emit,
        next_id: AtomicU32::new(1),
        jobs: Mutex::new(HashMap::new()),
        results: Mutex::new(VecDeque::new()),
    })))
}

fn forward<R: Runtime>(app: &AppHandle<R>, event: SpectrumEvent) {
    let result = match event {
        SpectrumEvent::Progress(dto) => emit_job_progress(app, dto),
        SpectrumEvent::Report(dto) => emit_spectrum_report(app, dto),
        SpectrumEvent::Notice(notice) => emit_notice(app, notice),
    };
    if let Err(error) = result {
        tracing::warn!(%error, "emitting a spectrum event failed");
    }
}

impl SpectrumService {
    /// `spectrum_analyze_start`: validates, then runs the job on its own thread; returns the
    /// job id at once.
    pub fn start_job(&self, request: SpectrumRequest) -> Result<u32, IpcError> {
        if !is_valid_fft_size(request.fft_size) {
            return Err(IpcError::new(
                IpcErrorCode::InvalidArgument,
                "error.spectrum.invalid_fft",
            ));
        }
        let source = self.0.documents.export_source()?;
        let (start, end) = resolve_range(request.range, source.len_samples)?;
        let mut sources = request.sources;
        if sources.is_empty() {
            sources.push(LoudnessSource::Processed);
        }
        let rack_model = sources
            .contains(&LoudnessSource::Processed)
            .then(|| self.0.engine.rack_model().unwrap_or_default());
        let config = OfflineConfig {
            sample_rate_hz: source.sample_rate_hz,
            fft_size: request.fft_size as usize,
            window: request.window,
        };
        let job_id = self.0.next_id.fetch_add(1, Ordering::Relaxed);
        let cancel = Arc::new(AtomicBool::new(false));
        self.0
            .jobs
            .lock()
            .unwrap()
            .insert(job_id, Arc::clone(&cancel));
        let inner = Arc::clone(&self.0);
        std::thread::Builder::new()
            .name("spectrum-job".into())
            .spawn(move || {
                let job = Job {
                    job_id,
                    start,
                    end,
                    sources,
                    rack_model,
                    config,
                };
                run_job(&inner, &job, &source, &cancel);
            })
            .map_err(|e| IpcError::internal(e.to_string()))?;
        Ok(job_id)
    }

    /// `spectrum_analyze_cancel`: best-effort; a no-op for an unknown or finished job.
    pub fn cancel_job(&self, job_id: u32) {
        if let Some(cancel) = self.0.jobs.lock().unwrap().get(&job_id) {
            cancel.store(true, Ordering::Relaxed);
        }
    }

    /// `spectrum_analyze_curve`: the `index`-th signal of a finished job as a `VXLT` frame.
    pub fn curve(&self, job_id: u32, index: u32) -> Result<Vec<u8>, IpcError> {
        let results = self.0.results.lock().unwrap();
        let outcome = results
            .iter()
            .find(|o| o.job_id == job_id)
            .ok_or_else(no_result)?;
        let curve = outcome.results.get(index as usize).ok_or_else(no_result)?;
        Ok(encode_curve(
            job_id,
            index,
            outcome.sample_rate_hz,
            &curve.analysis,
        ))
    }
}

struct Job {
    job_id: u32,
    start: u64,
    end: u64,
    sources: Vec<LoudnessSource>,
    rack_model: Option<RackModel>,
    config: OfflineConfig,
}

fn no_result() -> IpcError {
    IpcError::new(IpcErrorCode::InvalidArgument, "error.spectrum.no_result")
}

fn cancelled() -> IpcError {
    IpcError::new(IpcErrorCode::Cancelled, "error.cancelled")
}

fn resolve_range(range: Option<(u64, u64)>, len_samples: u64) -> Result<(u64, u64), IpcError> {
    match range {
        None if len_samples > 0 => Ok((0, len_samples)),
        Some((start, end)) if start < end && end <= len_samples => Ok((start, end)),
        _ => Err(IpcError::new(
            IpcErrorCode::InvalidArgument,
            "error.spectrum.invalid_range",
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

fn run_job(inner: &Inner, job: &Job, source: &crate::document::ExportSource, cancel: &AtomicBool) {
    // H-108: the owner's log had no line at all for spectrum-analyze, so a completed job and a
    // stuck one looked identical from the outside — H-96 gave export/normalize/bake this same
    // one start line + one finish line (at `info`, the level an operator actually reads).
    let started_at = Instant::now();
    tracing::info!(
        job_id = job.job_id,
        start = job.start,
        end = job.end,
        sources = ?job.sources,
        fft_size = job.config.fft_size,
        "spectrum analyze started"
    );
    let emit = Arc::clone(&inner.emit);
    let job_id = job.job_id;
    let mut last_progress: Option<Instant> = None;
    let result = read_range(source, job.start, job.end).and_then(|samples| {
        analyze_sources(
            &inner.registry,
            &samples,
            job.config,
            &job.sources,
            job.rack_model.as_ref(),
            cancel,
            |fraction| {
                if last_progress.is_some_and(|t| t.elapsed() < PROGRESS_INTERVAL) {
                    return;
                }
                last_progress = Some(Instant::now());
                emit(SpectrumEvent::Progress(JobProgressDto {
                    job_id,
                    kind: JobKind::SpectrumAnalyze,
                    state: JobState::Running,
                    fraction,
                }));
            },
        )
    });
    inner.jobs.lock().unwrap().remove(&job_id);

    let (state, fraction) = match &result {
        Ok(_) => (JobState::Done, 1.0),
        Err(err) if err.code == IpcErrorCode::Cancelled => (JobState::Cancelled, 0.0),
        Err(_) => (JobState::Failed, 0.0),
    };
    if let (JobState::Failed, Err(err)) = (state, &result) {
        (inner.emit)(SpectrumEvent::Notice(notice_from_error(err)));
    }
    tracing::info!(
        job_id,
        ?state,
        elapsed_ms = started_at.elapsed().as_millis() as u64,
        "spectrum analyze finished"
    );
    if let Ok(results) = result {
        let report = SpectrumReportDto {
            job_id,
            sample_rate_hz: job.config.sample_rate_hz,
            fft_size: job.config.fft_size as u32,
            window: job.config.window.into(),
            start_sample: job.start,
            end_sample: job.end,
            results: results
                .iter()
                .map(|r| SpectrumResultDto {
                    source: r.source.into(),
                    frames: r.analysis.frames,
                    has_noise: r.analysis.noise.is_some(),
                    report: VoiceReportDto::from(&r.analysis.report),
                })
                .collect(),
        };
        {
            let mut kept = inner.results.lock().unwrap();
            kept.push_back(Arc::new(JobOutcome {
                job_id,
                sample_rate_hz: job.config.sample_rate_hz,
                results,
            }));
            while kept.len() > KEPT_RESULTS {
                kept.pop_front();
            }
        }
        // The curves are fetchable before the UI hears the job is done.
        (inner.emit)(SpectrumEvent::Progress(JobProgressDto {
            job_id,
            kind: JobKind::SpectrumAnalyze,
            state,
            fraction,
        }));
        (inner.emit)(SpectrumEvent::Report(report));
    } else {
        (inner.emit)(SpectrumEvent::Progress(JobProgressDto {
            job_id,
            kind: JobKind::SpectrumAnalyze,
            state,
            fraction,
        }));
    }
}

/// Analyses `samples` once per requested signal (the rack render for "processed" comes first
/// in that signal's share of the progress). `progress` gets the overall fraction.
pub(crate) fn analyze_sources(
    registry: &Registry,
    samples: &[f32],
    config: OfflineConfig,
    sources: &[LoudnessSource],
    rack_model: Option<&RackModel>,
    cancel: &AtomicBool,
    mut progress: impl FnMut(f32),
) -> Result<Vec<CurveResult>, IpcError> {
    let n = sources.len().max(1) as f32;
    let mut results = Vec::with_capacity(sources.len());
    let default_model = RackModel::default();
    for (i, &source) in sources.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            return Err(cancelled());
        }
        let rendered;
        let signal: &[f32] = match source {
            LoudnessSource::Source => samples,
            LoudnessSource::Processed => {
                rendered = render_processed_to_buffer(
                    registry,
                    rack_model.unwrap_or(&default_model),
                    config.sample_rate_hz,
                    samples,
                    cancel,
                )?;
                &rendered
            }
        };
        let analysis = analyze_buffer(signal, config, |f| {
            progress((i as f32 + f) / n);
            !cancel.load(Ordering::Relaxed)
        })
        .ok_or_else(cancelled)?;
        results.push(CurveResult { source, analysis });
    }
    Ok(results)
}

fn db_f32(p: f64) -> f32 {
    power_db(p) as f32
}

/// Little-endian `VXLT` v1 (SPEC-007 §8.9): `"VXLT"`, u16 version 1, u16 header_len 36, u32
/// job_id, u32 index, u32 sample_rate_hz, u32 fft_size, u32 window, u32 bin_count, u32 flags
/// (bit 0 `HAS_NOISE`), then `f32[bin_count]` long-term average levels (dB, `-inf` allowed) and,
/// with `HAS_NOISE`, `f32[bin_count]` room-tone levels. Bin `k` is at `k · fs / fft_size` Hz.
pub fn encode_curve(job_id: u32, index: u32, sample_rate_hz: u32, a: &OfflineAnalysis) -> Vec<u8> {
    let bins = a.ltas.len();
    let noise = a.noise.as_ref();
    let mut b =
        Vec::with_capacity(VXLT_HEADER_LEN + 4 * bins * if noise.is_some() { 2 } else { 1 });
    b.extend_from_slice(b"VXLT");
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&(VXLT_HEADER_LEN as u16).to_le_bytes());
    b.extend_from_slice(&job_id.to_le_bytes());
    b.extend_from_slice(&index.to_le_bytes());
    b.extend_from_slice(&sample_rate_hz.to_le_bytes());
    b.extend_from_slice(&(a.fft_size as u32).to_le_bytes());
    b.extend_from_slice(&a.window.code().to_le_bytes());
    b.extend_from_slice(&(bins as u32).to_le_bytes());
    let flags = if noise.is_some() { VXLT_HAS_NOISE } else { 0 };
    b.extend_from_slice(&flags.to_le_bytes());
    for &p in &a.ltas {
        b.extend_from_slice(&db_f32(p).to_le_bytes());
    }
    if let Some(noise) = noise {
        for &p in noise {
            b.extend_from_slice(&db_f32(p).to_le_bytes());
        }
    }
    b
}

/// `spectrum_export_csv`: writes `contents` to `path` (a `.csv` extension is added when
/// missing), atomically (temp file + rename). Returns the path written.
pub fn write_csv(path: &Path, contents: &str) -> Result<PathBuf, IpcError> {
    let csv_error = |message: String| {
        IpcError::new(IpcErrorCode::Io, "error.spectrum.csv_write").with_param("message", message)
    };
    if contents.len() > MAX_CSV_BYTES {
        return Err(csv_error("too large".into()));
    }
    let mut target = path.to_path_buf();
    let is_csv = target
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("csv"));
    if !is_csv {
        let mut name = target.file_name().unwrap_or_default().to_os_string();
        name.push(".csv");
        target.set_file_name(name);
    }
    let mut tmp_name = target.file_name().unwrap_or_default().to_os_string();
    tmp_name.push(".tmp");
    let tmp = target.with_file_name(tmp_name);
    std::fs::write(&tmp, contents).map_err(|e| csv_error(e.to_string()))?;
    std::fs::rename(&tmp, &target).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        csv_error(e.to_string())
    })?;
    Ok(target)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use vox_engine::backend::fake::FakeBackend;
    use vox_engine::{Engine, EngineConfig};
    use vox_module_api::{ModuleRef, ModuleState, Version};
    use vox_rack::SlotModel;

    use crate::ipc::loudness_dto::LoudnessSourceDto;
    use crate::settings::BitDepth;

    use super::*;

    const FS: u32 = 48_000;

    fn registry() -> Registry {
        Registry::with_factories(vox_modules::builtin_factories()).unwrap()
    }

    fn tmp_dir(tag: &str) -> PathBuf {
        crate::test_util::tmp_dir(&format!("spectrum-{tag}"))
    }

    fn gain_model(db: f64) -> RackModel {
        let slot = SlotModel::new(
            &ModuleRef {
                id: "org.powervoice.gain".into(),
                version: Version::new(1, 0, 0),
            },
            false,
            &ModuleState {
                format_version: 1,
                params: BTreeMap::from([("gain_db".to_string(), db)]),
                blob: None,
            },
        );
        RackModel { slots: vec![slot] }
    }

    fn config(fft: usize) -> OfflineConfig {
        OfflineConfig {
            sample_rate_hz: FS,
            fft_size: fft,
            window: WindowKind::Hann,
        }
    }

    #[test]
    fn source_vs_processed_differ_by_the_rack() {
        // A bin-centred −20 dBFS tone through a +6 dB gain: the processed average reads 6 dB
        // higher at that bin, the source average doesn't move.
        let fft = 4096;
        let freq = 64.0 * f64::from(FS) / fft as f64;
        let x = vox_testkit::signal::sine(freq, -20.0, 2.0, FS).unwrap();
        let results = analyze_sources(
            &registry(),
            &x,
            config(fft),
            &[LoudnessSource::Source, LoudnessSource::Processed],
            Some(&gain_model(6.0)),
            &AtomicBool::new(false),
            |_| {},
        )
        .unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].source, LoudnessSource::Source);
        let src = power_db(results[0].analysis.ltas[64]);
        let proc = power_db(results[1].analysis.ltas[64]);
        assert!((src - (-20.0)).abs() < 0.1, "source {src}");
        assert!((proc - src - 6.0).abs() < 0.05, "processed {proc}");
    }

    #[test]
    fn a_cancelled_job_reports_cancelled() {
        let x = vox_testkit::signal::sine(440.0, -20.0, 2.0, FS).unwrap();
        let cancel = AtomicBool::new(true);
        let err = analyze_sources(
            &registry(),
            &x,
            config(1024),
            &[LoudnessSource::Source],
            None,
            &cancel,
            |_| {},
        )
        .err()
        .unwrap();
        assert_eq!(err.code, IpcErrorCode::Cancelled);
    }

    /// H-108: end-to-end regression through the real job path (never the preview harness's mock,
    /// which always uses job id 11 and perfect event order) for both of the ticket's hypotheses,
    /// on the exact scenario the owner hit — an **unsaved recording** (`begin_recording`/
    /// `commit_take`, never `open`, so `doc.path` stays `None`) with **no rack** slots at all,
    /// analysing "processed" (the analyzer's own default `averageSource`):
    /// 1. every `job_progress` tick and the terminal `spectrum_report` must carry the *same*
    ///    `job_id` `start_job` returned — a mismatch would explain `AnalyzerPanel.svelte`'s
    ///    completion effect (gated on `averageReport?.job_id === job.jobId`) waiting forever;
    /// 2. the job must actually reach `Done` with a report that has an entry for the requested
    ///    source, so `diag.averages.find(a => a.source === diag.averageSource)` never comes up
    ///    empty on this scenario (the other way the completion effect can hang forever).
    #[test]
    fn unsaved_recording_with_no_rack_reports_under_the_same_job_id() {
        let dir = tmp_dir("unsaved-recording");
        let fake = FakeBackend::new(1);
        let engine_registry =
            Arc::new(Registry::with_factories(vox_modules::builtin_factories()).unwrap());
        let engine = Engine::start(EngineConfig::new(Arc::new(fake), engine_registry)).unwrap();
        let documents =
            crate::document::DocumentService::new(dir.join("sessions"), engine.handle());

        // An unsaved recording: never `open`, so the document has no path (matches the ticket's
        // "unsaved recording" repro exactly) — and no rack slot is ever added, so the "processed"
        // render below goes through `RackModel::default()`.
        let (mut capture, _info) = documents
            .begin_recording(FS, BitDepth::Bit24, false)
            .unwrap();
        let samples = vox_testkit::signal::sine(440.0, -20.0, 0.5, FS).unwrap();
        capture.append(&samples).unwrap();
        documents.commit_take(&capture.finish(), &[]).unwrap();

        let events: Arc<Mutex<Vec<SpectrumEvent>>> = Arc::new(Mutex::new(Vec::new()));
        let events_for_emit = Arc::clone(&events);
        let emit: SpectrumEmitter = Arc::new(move |event| {
            events_for_emit.lock().unwrap().push(event);
        });
        let inner = Arc::new(Inner {
            documents,
            engine: engine.handle(),
            registry: Arc::new(registry()),
            emit,
            next_id: AtomicU32::new(1),
            jobs: Mutex::new(HashMap::new()),
            results: Mutex::new(VecDeque::new()),
        });
        let service = SpectrumService(inner);

        let job_id = service
            .start_job(SpectrumRequest {
                range: None,
                sources: vec![LoudnessSource::Processed],
                fft_size: 1024,
                window: WindowKind::Hann,
            })
            .unwrap();

        let mut report = None;
        for _ in 0..500 {
            report = events.lock().unwrap().iter().find_map(|e| match e {
                SpectrumEvent::Report(r) => Some(r.clone()),
                _ => None,
            });
            if report.is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let report = report.expect(
            "spectrum_report never arrived — the average job on an unsaved recording with no \
             rack got stuck",
        );

        let seen = events.lock().unwrap();
        let progress_ids: Vec<u32> = seen
            .iter()
            .filter_map(|e| match e {
                SpectrumEvent::Progress(p) => Some(p.job_id),
                _ => None,
            })
            .collect();
        assert!(!progress_ids.is_empty(), "no job_progress events at all");
        assert!(
            progress_ids.iter().all(|&id| id == job_id),
            "a job_progress event carried a different job_id than spectrum_analyze_start \
             returned: {progress_ids:?} vs {job_id}"
        );
        assert_eq!(
            report.job_id, job_id,
            "spectrum_report carried a different job_id than spectrum_analyze_start returned"
        );
        let final_state = seen.iter().rev().find_map(|e| match e {
            SpectrumEvent::Progress(p) if p.job_id == job_id => Some(p.state),
            _ => None,
        });
        assert_eq!(
            final_state,
            Some(JobState::Done),
            "the job never reached Done"
        );
        assert_eq!(report.results.len(), 1);
        assert_eq!(report.results[0].source, LoudnessSourceDto::Processed);
    }

    #[test]
    fn progress_is_monotonic_across_signals() {
        let x = vox_testkit::signal::sine(440.0, -20.0, 6.0, FS).unwrap();
        let mut seen = Vec::new();
        analyze_sources(
            &registry(),
            &x,
            config(2048),
            &[LoudnessSource::Source, LoudnessSource::Processed],
            None,
            &AtomicBool::new(false),
            |f| seen.push(f),
        )
        .unwrap();
        assert!(seen.windows(2).all(|w| w[1] >= w[0] - 1e-6), "{seen:?}");
        assert!(seen.iter().any(|&f| f > 0.5));
        assert!((seen.last().copied().unwrap() - 1.0).abs() < 1e-6);
    }

    #[test]
    fn vxlt_layout() {
        let a = OfflineAnalysis {
            fft_size: 8,
            window: WindowKind::BlackmanHarris,
            enbw_bins: 2.0,
            ltas: vec![1.0, 0.25, 0.0, 0.01, 1e-4],
            noise: Some(vec![0.0; 5]),
            frames: 3,
            report: Default::default(),
        };
        let b = encode_curve(9, 1, FS, &a);
        assert_eq!(&b[..4], b"VXLT");
        assert_eq!(b.len(), VXLT_HEADER_LEN + 2 * 4 * 5);
        let u32_at = |o: usize| u32::from_le_bytes(b[o..o + 4].try_into().unwrap());
        let f32_at = |o: usize| f32::from_le_bytes(b[o..o + 4].try_into().unwrap());
        assert_eq!(u32_at(8), 9);
        assert_eq!(u32_at(12), 1);
        assert_eq!(u32_at(16), FS);
        assert_eq!(u32_at(20), 8);
        assert_eq!(u32_at(24), 1);
        assert_eq!(u32_at(28), 5);
        assert_eq!(u32_at(32), VXLT_HAS_NOISE);
        assert!(f32_at(36).abs() < 1e-9);
        assert!((f32_at(40) - (-6.0206)).abs() < 1e-3);
        assert!(f32_at(44).is_infinite() && f32_at(44) < 0.0);
    }

    #[test]
    fn range_validation() {
        assert_eq!(resolve_range(None, 100).unwrap(), (0, 100));
        assert!(resolve_range(None, 0).is_err());
        assert_eq!(resolve_range(Some((10, 20)), 100).unwrap(), (10, 20));
        assert!(resolve_range(Some((20, 10)), 100).is_err());
        assert!(resolve_range(Some((0, 101)), 100).is_err());
    }

    #[test]
    fn csv_is_written_atomically_with_its_extension() {
        let dir = std::env::temp_dir().join(format!("pv-spectrum-csv-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let written = write_csv(&dir.join("voice"), "frequency_hz,level_db\n100,-20\n").unwrap();
        assert_eq!(written, dir.join("voice.csv"));
        assert_eq!(
            std::fs::read_to_string(&written).unwrap(),
            "frequency_hz,level_db\n100,-20\n"
        );
        assert!(!dir.join("voice.csv.tmp").exists());
        let kept = write_csv(&dir.join("A.CSV"), "x").unwrap();
        assert_eq!(kept, dir.join("A.CSV"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// H-42: the golden `VXLT` curve frame for the TS decoder test (SPEC-007 §8.9).
    #[test]
    fn export_bindings_vxlt_fixture() {
        let a = OfflineAnalysis {
            fft_size: 4,
            window: WindowKind::Hann,
            enbw_bins: 1.5,
            ltas: vec![1.0, 0.25, 0.0],
            noise: Some(vec![1e-6, 1e-8, 0.0]),
            frames: 2,
            report: Default::default(),
        };
        let hex: String = encode_curve(5, 1, 44_100, &a)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        let ts = format!(
            "// Generated by the `export_bindings_vxlt_fixture` test (src-tauri, `just gen-types`). Do not edit.\n\
             \n\
             /** A `VXLT` curve frame encoded by Rust (`spectrum::encode_curve`) for the TS decoder contract test. */\n\
             export const VXLT_FIXTURE_HEX =\n  \"{hex}\";\n",
        );
        let dir = std::env::var_os("TS_RS_EXPORT_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("bindings"));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("vxlt_fixture.ts"), ts).unwrap();
    }
}

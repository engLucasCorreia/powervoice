//! Loudness analysis job (S4-01): integrated/short-term/momentary loudness, LRA, sample and true
//! peak (`vox_dsp::loudness`) of a document range — either its raw source samples, or the
//! rack-processed signal through the same offline chain code export/bake use
//! (`vox_rack::offline`, ADR-001 §5's "one rack code path"; ADR-001 §4 lists "processed-output
//! analysis" as one of `engine`'s jobs, alongside export/bake/ACX — this ticket's job lives here
//! instead, mirroring S4-04's export job, which the same ADR row names but which also lives in
//! `src-tauri` today, MEMORY.md).
//!
//! Runs as a cancellable job on its own thread, reporting `job_progress` like S4-04's export job
//! (`JobKind::LoudnessAnalyze`), plus a `loudness_report` event carrying the finished
//! [`vox_dsp::loudness::LoudnessReport`] once done — `job_progress`'s `fraction`/`state` alone
//! can't carry the report itself.
//!
//! **Deviation (ticket report, mirrors export.rs's own note):** "processed" mode uses whatever
//! `RackModel` is live when the job starts (`EngineHandle::rack_model()`, added by S3-01/T-103 for
//! this handoff) — no automation, matching `vox_rack::offline::render`'s "static parameters"
//! contract. LUFS normalize (`vox_project::normalize_lufs`, wired through
//! `DocumentService::edit_normalize_lufs`) measures the raw *source* samples directly instead
//! (SPEC-010's write path, `project` alone, no rack — see `crates/project/src/normalize.rs`'s
//! module docs), so it needs none of this file.
//!
//! S4-03 (ACX check) reuses this same service and its render plumbing (`LoudnessService::
//! run_acx_check`, `render_processed_to_buffer`) rather than duplicating the read/render setup —
//! `acx_check` is a plain synchronous command (no job id, no `job_progress`/cancel), unlike the
//! analysis job above, since the ticket asks for "Command `acx_check` → report DTO" directly.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use tauri::{AppHandle, Runtime};
use vox_dsp::acx::AcxReport;
use vox_dsp::loudness::{LoudnessError, LoudnessMeter, LoudnessReport};
use vox_engine::EngineHandle;
use vox_project::{CHUNK_SAMPLES, SnapshotReader};
use vox_rack::{RackModel, Registry, Transport};

use crate::document::{DocumentService, ExportSource};
use crate::ipc::loudness_dto::LoudnessReportDto;
use crate::ipc::{
    IpcError, IpcErrorCode, JobKind, JobProgressDto, JobState, Notice, NoticeLevel,
    emit_job_progress, emit_loudness_report, emit_notice,
};

/// Which signal `loudness_analyze_start` measures (ticket: "measuring the processed output ...
/// by default, with a source toggle").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoudnessSource {
    /// The rack-rendered signal, as heard/exported — the default.
    Processed,
    /// The raw document samples, bypassing the rack entirely.
    Source,
}

/// `LoudnessService::start_job`'s argument.
pub struct LoudnessRequest {
    /// `[start, end)` samples; `None` = the whole document.
    pub range: Option<(u64, u64)>,
    pub source: LoudnessSource,
}

enum LoudnessEvent {
    Progress(JobProgressDto),
    Report { job_id: u32, report: LoudnessReport },
    Notice(Notice),
}

type LoudnessEmitter = Arc<dyn Fn(LoudnessEvent) + Send + Sync>;

struct JobHandle {
    cancel: Arc<AtomicBool>,
}

struct Inner {
    documents: DocumentService,
    engine: EngineHandle,
    registry: Arc<Registry>,
    emit: LoudnessEmitter,
    next_id: AtomicU32,
    jobs: Mutex<HashMap<u32, JobHandle>>,
}

/// Cheaply cloneable (an `Arc` inside), like [`crate::export::ExportService`].
#[derive(Clone)]
pub struct LoudnessService(Arc<Inner>);

/// Builds the service (composition root: its own `Registry` instance, like `ExportService` —
/// modules are stateless per-instance).
pub fn start<R: Runtime>(
    app: AppHandle<R>,
    documents: DocumentService,
    engine: EngineHandle,
) -> anyhow::Result<LoudnessService> {
    let registry = crate::plugins::registry()?;
    let emit: LoudnessEmitter = Arc::new(move |event| forward(&app, event));
    Ok(LoudnessService(Arc::new(Inner {
        documents,
        engine,
        registry,
        emit,
        next_id: AtomicU32::new(1),
        jobs: Mutex::new(HashMap::new()),
    })))
}

fn forward<R: Runtime>(app: &AppHandle<R>, event: LoudnessEvent) {
    let result = match event {
        LoudnessEvent::Progress(dto) => emit_job_progress(app, dto),
        LoudnessEvent::Report { job_id, report } => {
            emit_loudness_report(app, LoudnessReportDto::from_report(job_id, report))
        }
        LoudnessEvent::Notice(notice) => emit_notice(app, notice),
    };
    if let Err(error) = result {
        tracing::warn!(%error, "emitting a loudness event failed");
    }
}

impl LoudnessService {
    /// `loudness_analyze_start`: validates the range against the current document, then runs the
    /// job on its own thread. Returns the job id immediately; `job_progress` events (tagged with
    /// it) report progress/outcome, and a `loudness_report` event carries the finished report.
    pub fn start_job(&self, request: LoudnessRequest) -> Result<u32, IpcError> {
        let source = self.0.documents.export_source()?;
        let (start, end) = resolve_range(request.range, source.len_samples)?;
        let rack_model = match request.source {
            LoudnessSource::Processed => Some(self.0.engine.rack_model().unwrap_or_default()),
            LoudnessSource::Source => None,
        };
        let job_id = self.0.next_id.fetch_add(1, Ordering::Relaxed);
        let cancel = Arc::new(AtomicBool::new(false));
        self.0.jobs.lock().unwrap().insert(
            job_id,
            JobHandle {
                cancel: Arc::clone(&cancel),
            },
        );
        let inner = Arc::clone(&self.0);
        std::thread::Builder::new()
            .name("loudness-job".into())
            .spawn(move || {
                run_job(inner, job_id, source, start, end, rack_model, cancel);
            })
            .map_err(|e| IpcError::internal(e.to_string()))?;
        Ok(job_id)
    }

    /// `loudness_analyze_cancel`: best-effort, like `ExportService::cancel_job`. A no-op for an
    /// already-finished or unknown job id.
    pub fn cancel_job(&self, job_id: u32) {
        if let Some(job) = self.0.jobs.lock().unwrap().get(&job_id) {
            job.cancel.store(true, Ordering::Relaxed);
        }
    }

    /// `acx_check` (S4-03): synchronous — meant to run inside `spawn_blocking`, not its own job
    /// thread (no progress/cancel, per the ticket). Always the whole document: ACX submissions
    /// are whole chapters, not selections, unlike the loudness analysis job above. Reads the raw
    /// samples, renders them through the live rack for `LoudnessSource::Processed` (same
    /// `EngineHandle::rack_model()` convention as `start_job`), then evaluates the result against
    /// the ACX rules (`vox_dsp::acx::evaluate`).
    pub fn run_acx_check(&self, source: LoudnessSource) -> Result<AcxReport, IpcError> {
        let export_source = self.0.documents.export_source()?;
        let samples = read_range(&export_source, 0, export_source.len_samples)?;
        let cancel = AtomicBool::new(false);
        let buffer = match source {
            LoudnessSource::Source => samples,
            LoudnessSource::Processed => {
                let rack_model = self.0.engine.rack_model().unwrap_or_default();
                render_processed_to_buffer(
                    &self.0.registry,
                    &rack_model,
                    export_source.sample_rate_hz,
                    &samples,
                    &cancel,
                )?
            }
        };
        Ok(vox_dsp::acx::evaluate(
            &buffer,
            export_source.sample_rate_hz,
        ))
    }
}

fn resolve_range(range: Option<(u64, u64)>, len_samples: u64) -> Result<(u64, u64), IpcError> {
    match range {
        None => Ok((0, len_samples)),
        Some((start, end)) if start < end && end <= len_samples => Ok((start, end)),
        Some(_) => Err(IpcError::new(
            IpcErrorCode::InvalidArgument,
            "error.loudness.invalid_range",
        )),
    }
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

fn project_read_error(err: vox_project::ProjectError) -> IpcError {
    IpcError::new(IpcErrorCode::Io, "error.loudness.read").with_param("message", err.to_string())
}

fn rack_error(err: vox_rack::offline::RenderError) -> IpcError {
    IpcError::new(IpcErrorCode::Internal, "error.loudness.rack")
        .with_param("message", err.to_string())
}

fn loudness_error(err: LoudnessError) -> IpcError {
    IpcError::new(IpcErrorCode::Internal, "error.internal").with_param("message", err.to_string())
}

fn notice_from_error(err: &IpcError) -> Notice {
    let mut notice = Notice::toast(NoticeLevel::Error, err.key.clone());
    for (name, value) in &err.params {
        notice = notice.with_param(name.clone(), value.clone());
    }
    notice
}

/// The same "whole buffer in RAM" read `export.rs::read_range` uses (H-02 backlog item to stream
/// it — shared with export, not duplicated fresh here).
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

fn run_job(
    inner: Arc<Inner>,
    job_id: u32,
    source: ExportSource,
    start: u64,
    end: u64,
    rack_model: Option<RackModel>,
    cancel: Arc<AtomicBool>,
) {
    let emit = Arc::clone(&inner.emit);
    let result = run_pipeline(
        &inner.registry,
        &source,
        start,
        end,
        rack_model.as_ref(),
        &cancel,
        |fraction| {
            emit(LoudnessEvent::Progress(JobProgressDto {
                job_id,
                kind: JobKind::LoudnessAnalyze,
                state: JobState::Running,
                fraction,
            }));
        },
    );
    inner.jobs.lock().unwrap().remove(&job_id);

    let (state, fraction) = match &result {
        Ok(_) => (JobState::Done, 1.0),
        Err(err) if err.code == IpcErrorCode::Cancelled => (JobState::Cancelled, 0.0),
        Err(_) => (JobState::Failed, 0.0),
    };
    (inner.emit)(LoudnessEvent::Progress(JobProgressDto {
        job_id,
        kind: JobKind::LoudnessAnalyze,
        state,
        fraction,
    }));
    match result {
        Ok(report) => (inner.emit)(LoudnessEvent::Report { job_id, report }),
        Err(err) => (inner.emit)(LoudnessEvent::Notice(notice_from_error(&err))),
    }
}

/// Read → (optionally rack-render) → stream into a [`LoudnessMeter`]. `rack_model: None` is
/// "source" mode (the rack is skipped entirely, not just given an empty model); `Some(model)` is
/// "processed" mode, rendered exactly like `vox_rack::offline::render` (time-aligned latency
/// trim), but streamed straight into the meter instead of materializing the rendered signal.
fn run_pipeline(
    registry: &Registry,
    source: &ExportSource,
    start: u64,
    end: u64,
    rack_model: Option<&RackModel>,
    cancel: &AtomicBool,
    progress: impl FnMut(f32),
) -> Result<LoudnessReport, IpcError> {
    check_cancelled(cancel)?;
    let samples = read_range(source, start, end)?;

    check_cancelled(cancel)?;
    match rack_model {
        None => analyze_source(&samples, source.sample_rate_hz, cancel, progress),
        Some(model) => analyze_processed(
            registry,
            model,
            source.sample_rate_hz,
            &samples,
            cancel,
            progress,
        ),
    }
}

/// "Source" mode: streams the raw samples straight into a [`LoudnessMeter`] in
/// [`CHUNK_SAMPLES`]-sized hops (bounds memory the same way `vox_project::normalize`'s scan
/// passes do; here it's for progress, since the whole buffer is already in RAM).
fn analyze_source(
    samples: &[f32],
    sample_rate_hz: u32,
    cancel: &AtomicBool,
    mut progress: impl FnMut(f32),
) -> Result<LoudnessReport, IpcError> {
    let mut meter = LoudnessMeter::new(sample_rate_hz).map_err(loudness_error)?;
    if samples.is_empty() {
        return meter.finish().map_err(loudness_error);
    }
    let mut pos = 0usize;
    while pos < samples.len() {
        check_cancelled(cancel)?;
        let n = CHUNK_SAMPLES.min(samples.len() - pos);
        meter.push(&samples[pos..pos + n]).map_err(loudness_error)?;
        pos += n;
        progress(pos as f32 / samples.len() as f32);
    }
    meter.finish().map_err(loudness_error)
}

/// "Processed" mode: the same block loop as `export.rs::render_with_progress` (4096-frame
/// blocks, per-block cancellation, latency trim), but instead of accumulating the rendered
/// output in a `Vec`, each trimmed block is pushed straight into a [`LoudnessMeter`] — the
/// analysis job never needs the whole rendered signal in memory at once.
fn analyze_processed(
    registry: &Registry,
    model: &RackModel,
    sample_rate_hz: u32,
    samples: &[f32],
    cancel: &AtomicBool,
    progress: impl FnMut(f32),
) -> Result<LoudnessReport, IpcError> {
    let mut meter = LoudnessMeter::new(sample_rate_hz).map_err(loudness_error)?;
    render_processed(
        registry,
        model,
        sample_rate_hz,
        samples,
        cancel,
        progress,
        |chunk| meter.push(chunk).map_err(loudness_error),
    )?;
    meter.finish().map_err(loudness_error)
}

/// Renders `samples` through `model` (same time-aligned offline chain as `vox_rack::offline`,
/// SPEC-012 §2.8) into a single `Vec<f32>` — used by the ACX check (S4-03), which needs the whole
/// rendered waveform for RMS/noise-floor measurement rather than a streamed loudness meter.
fn render_processed_to_buffer(
    registry: &Registry,
    model: &RackModel,
    sample_rate_hz: u32,
    samples: &[f32],
    cancel: &AtomicBool,
) -> Result<Vec<f32>, IpcError> {
    let mut buffer = Vec::with_capacity(samples.len());
    render_processed(
        registry,
        model,
        sample_rate_hz,
        samples,
        cancel,
        |_fraction| {},
        |chunk| {
            buffer.extend_from_slice(chunk);
            Ok(())
        },
    )?;
    Ok(buffer)
}

/// The shared block-render loop behind both [`analyze_processed`] (sink = push into a
/// [`LoudnessMeter`]) and [`render_processed_to_buffer`] (sink = extend a `Vec`): 4096-frame
/// blocks (`vox_rack::OFFLINE_BLOCK`), per-block cancellation, and the same latency trim
/// `export.rs::render_with_progress` uses (`out.drain(..latency); out.truncate(input.len())`,
/// done incrementally here instead of on a fully materialized buffer).
fn render_processed(
    registry: &Registry,
    model: &RackModel,
    sample_rate_hz: u32,
    samples: &[f32],
    cancel: &AtomicBool,
    mut progress: impl FnMut(f32),
    mut sink: impl FnMut(&[f32]) -> Result<(), IpcError>,
) -> Result<(), IpcError> {
    let _fp = vox_dsp::fp::DenormalGuard::new();
    let mut chain = vox_rack::offline::build_chain(registry, model, f64::from(sample_rate_hz))
        .map_err(rack_error)?;
    let latency = chain.latency_samples() as usize;
    let total = samples.len() + latency;
    if total == 0 {
        chain.deactivate();
        return Ok(());
    }
    let block = vox_rack::OFFLINE_BLOCK as usize;
    let mut inbuf = vec![0.0f32; block];
    let mut outbuf = vec![0.0f32; block];
    let mut pos = 0usize;
    while pos < total {
        if cancel.load(Ordering::Relaxed) {
            chain.deactivate();
            return Err(cancelled());
        }
        let n = block.min(total - pos);
        let avail = samples.len().saturating_sub(pos).min(n);
        inbuf[..avail].copy_from_slice(&samples[pos..pos + avail]);
        inbuf[avail..n].fill(0.0);
        chain.process(
            Transport {
                playing: true,
                position_samples: Some(pos as u64),
            },
            &inbuf[..n],
            &mut outbuf[..n],
        );
        if let Some((index, _reason)) = chain.take_failure() {
            let name = chain.slot_name(index).unwrap_or_default().to_owned();
            chain.deactivate();
            return Err(
                IpcError::new(IpcErrorCode::Internal, "error.loudness.slot_failed")
                    .with_param("slot", name),
            );
        }
        chain.drain_events(|_| {});

        // Latency trim, streamed: only the output positions in `[latency, latency +
        // samples.len())` are the time-aligned signal (mirrors `render_with_progress`'s
        // `out.drain(..latency); out.truncate(input.len())`, done incrementally).
        let want_start = pos.max(latency);
        let want_end = (pos + n).min(latency + samples.len());
        if want_start < want_end {
            sink(&outbuf[(want_start - pos)..(want_end - pos)])?;
        }
        pos += n;
        progress(pos as f32 / total as f32);
    }
    chain.deactivate();
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use vox_module_api::{ModuleRef, ModuleState, Version};
    use vox_rack::SlotModel;

    use super::*;

    fn registry() -> Registry {
        Registry::with_factories(vox_modules::builtin_factories()).unwrap()
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

    #[test]
    fn resolve_range_validates_bounds() {
        assert_eq!(resolve_range(None, 100).unwrap(), (0, 100));
        assert_eq!(resolve_range(Some((10, 20)), 100).unwrap(), (10, 20));
        assert!(resolve_range(Some((20, 10)), 100).is_err());
        assert!(resolve_range(Some((0, 101)), 100).is_err());
    }

    /// `analyze_source` must agree with the one-shot `vox_dsp::loudness::measure` on the same
    /// buffer (the hop-based streaming shouldn't change the answer).
    #[test]
    fn analyze_source_matches_a_one_shot_measurement() {
        let samples = vox_testkit::signal::sine(1000.0, -20.0, 2.0, 48_000).unwrap();
        let streamed = analyze_source(&samples, 48_000, &AtomicBool::new(false), |_| {}).unwrap();
        let one_shot = vox_dsp::loudness::measure(&samples, 48_000).unwrap();
        assert!((streamed.integrated_lufs - one_shot.integrated_lufs).abs() < 1e-6);
        assert!((streamed.true_peak_dbtp - one_shot.true_peak_dbtp).abs() < 1e-6);
    }

    /// "Processed" mode through an empty rack (passthrough, zero latency) must match "source"
    /// mode on the same buffer.
    #[test]
    fn analyze_processed_through_an_empty_rack_matches_source() {
        let samples = vox_testkit::signal::sine(1000.0, -20.0, 2.0, 48_000).unwrap();
        let reg = registry();
        let source = analyze_source(&samples, 48_000, &AtomicBool::new(false), |_| {}).unwrap();
        let processed = analyze_processed(
            &reg,
            &RackModel::default(),
            48_000,
            &samples,
            &AtomicBool::new(false),
            |_| {},
        )
        .unwrap();
        assert!((processed.integrated_lufs - source.integrated_lufs).abs() < 1e-4);
        assert!((processed.true_peak_dbtp - source.true_peak_dbtp).abs() < 1e-4);
    }

    /// Ticket AC: "processed vs source toggle differs by the rack gain (Gain −6 dB → 6.0 ± 0.1
    /// LU)".
    #[test]
    fn analyze_processed_through_gain_minus_6_db_differs_by_six_lu() {
        let samples = vox_testkit::signal::sine(1000.0, -20.0, 2.0, 48_000).unwrap();
        let reg = registry();
        let source = analyze_source(&samples, 48_000, &AtomicBool::new(false), |_| {}).unwrap();
        let processed = analyze_processed(
            &reg,
            &gain_model(-6.0),
            48_000,
            &samples,
            &AtomicBool::new(false),
            |_| {},
        )
        .unwrap();
        let delta_lu = source.integrated_lufs - processed.integrated_lufs;
        assert!(
            (delta_lu - 6.0).abs() <= 0.1,
            "expected a 6.0 +/- 0.1 LU drop, got {delta_lu:.4}"
        );
        let delta_dbtp = source.true_peak_dbtp - processed.true_peak_dbtp;
        assert!((delta_dbtp - 6.0).abs() <= 0.1);
    }

    #[test]
    fn analyze_processed_is_cancellable() {
        let samples = vox_testkit::signal::sine(1000.0, -20.0, 5.0, 48_000).unwrap();
        let reg = registry();
        let cancel = AtomicBool::new(true);
        let err = analyze_processed(
            &reg,
            &RackModel::default(),
            48_000,
            &samples,
            &cancel,
            |_| {},
        )
        .unwrap_err();
        assert_eq!(err.code, IpcErrorCode::Cancelled);
    }

    #[test]
    fn analyze_source_is_cancellable() {
        let samples = vox_testkit::signal::sine(1000.0, -20.0, 5.0, 48_000).unwrap();
        let cancel = AtomicBool::new(true);
        let err = analyze_source(&samples, 48_000, &cancel, |_| {}).unwrap_err();
        assert_eq!(err.code, IpcErrorCode::Cancelled);
    }

    #[test]
    fn analyze_processed_reports_progress_up_to_one() {
        let samples = vox_testkit::signal::sine(1000.0, -20.0, 1.0, 48_000).unwrap();
        let reg = registry();
        let mut last = 0.0f32;
        analyze_processed(
            &reg,
            &gain_model(-3.0),
            48_000,
            &samples,
            &AtomicBool::new(false),
            |fraction| last = fraction,
        )
        .unwrap();
        assert!((last - 1.0).abs() < 1e-6);
    }

    /// `render_processed_to_buffer` through an empty rack (passthrough, zero latency) must be
    /// bit-exact with the input — the ACX check's "processed" path shares this helper.
    #[test]
    fn render_processed_to_buffer_through_an_empty_rack_matches_the_input() {
        let samples = vox_testkit::signal::sine(1000.0, -20.0, 1.0, 48_000).unwrap();
        let reg = registry();
        let rendered = render_processed_to_buffer(
            &reg,
            &RackModel::default(),
            48_000,
            &samples,
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(rendered.len(), samples.len());
        for (a, b) in rendered.iter().zip(samples.iter()) {
            assert!((a - b).abs() < 1e-6);
        }
    }

    #[test]
    fn render_processed_to_buffer_is_cancellable() {
        let samples = vox_testkit::signal::sine(1000.0, -20.0, 5.0, 48_000).unwrap();
        let reg = registry();
        let cancel = AtomicBool::new(true);
        let err =
            render_processed_to_buffer(&reg, &RackModel::default(), 48_000, &samples, &cancel)
                .unwrap_err();
        assert_eq!(err.code, IpcErrorCode::Cancelled);
    }

    /// Ticket AC: "processed vs source toggle reflects a Gain −6 dB rack" — for the ACX check
    /// specifically: measuring the same buffer's RMS before/after a `[Gain −6 dB]` render must
    /// differ by exactly 6 dB (mirrors the loudness job's own `analyze_processed_through_gain_
    /// minus_6_db_differs_by_six_lu` test, one level down at the `vox_dsp::acx` evaluation).
    #[test]
    fn acx_evaluate_on_processed_vs_source_differs_by_the_rack_gain() {
        let samples = vox_testkit::signal::white_noise(1, -20.0, 2.0, 48_000).unwrap();
        let reg = registry();
        let processed = render_processed_to_buffer(
            &reg,
            &gain_model(-6.0),
            48_000,
            &samples,
            &AtomicBool::new(false),
        )
        .unwrap();

        let source_report = vox_dsp::acx::evaluate(&samples, 48_000);
        let processed_report = vox_dsp::acx::evaluate(&processed, 48_000);
        let delta_db =
            source_report.rms.measured_db.unwrap() - processed_report.rms.measured_db.unwrap();
        assert!(
            (delta_db - 6.0).abs() <= 0.1,
            "expected a 6.0 +/- 0.1 dB drop, got {delta_db:.4}"
        );
    }
}

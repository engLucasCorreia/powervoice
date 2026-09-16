//! Export job (S4-04, H-08): renders the **live rack** (`EngineHandle::rack_model()`, snapshotted
//! synchronously in [`ExportService::start_job`] before the job thread spawns — later rack edits
//! never affect a running export) over the whole document or a sample range through
//! `vox_rack::offline` (SPEC-012 §2.8, time-aligned), converts sample rate
//! (`vox_dsp::resample::resample_offline`, f64) and bit depth/format via S4-02's encoders
//! (`vox_io`), and writes atomically. Runs as a cancellable job on its own thread, reporting
//! `job_progress` (ADR-003) at each rack-render block plus phase boundaries; never touches the
//! document (D-019 — only Save/Save As write it).
//!
//! **T-602:** the rack render is `vox_engine::bake::render_document_range` — the one render path
//! export shares with Bake rack, so a bake is bit-identical to an export of the same range and
//! rack. A selection export therefore renders with the T-602 context (pre-roll from the audio
//! before the selection, post-roll from the audio after it; SPEC-012 §2.8 amendment); a
//! whole-file export is unchanged (SPEC-012's whole-file render).
//!
//! **H-08:** the rack panel (S3-01) and selection (S2-01) have both landed, so the "rack is
//! always empty" / "whole file only" deviations this module used to carry are resolved: the
//! dialog's Whole file/Selection choice sends `range` through unchanged (it was already plumbed
//! end to end), and SPEC-014's "Output noise only" pre-export confirmation is UI-side
//! (`ui/src/lib/export/export.svelte.ts`), reading the same live `RackStateDto` the rack panel
//! shows.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use tauri::{AppHandle, Runtime};
use vox_engine::EngineHandle;
use vox_engine::bake::{DocumentRange, RangeRenderError, render_document_range_to_vec};
use vox_engine::spectro::SpectroService;
use vox_project::CancelToken;
use vox_rack::{RackModel, Registry};

use crate::document::{DocumentService, ExportSource};
use crate::ipc::{
    IpcError, IpcErrorCode, JobKind, JobProgressDto, JobState, Notice, NoticeLevel,
    emit_job_progress, emit_notice,
};

/// Output format + its settings (mirrors `vox_io`'s per-format types).
#[derive(Debug, Clone, Copy)]
pub enum ExportFormat {
    Wav(vox_io::BitDepth),
    Flac(vox_io::FlacBitDepth),
    Mp3(vox_io::Mp3Settings),
}

/// `ExportService::start_job`'s argument.
pub struct ExportRequest {
    pub path: PathBuf,
    pub format: ExportFormat,
    pub sample_rate_hz: u32,
    /// `[start, end)` samples; `None` = the whole document.
    pub range: Option<(u64, u64)>,
}

/// What the service tells the UI (mirrors `audio.rs`/`recording.rs`'s own event enum + emitter
/// pattern, built once at startup from a concrete `AppHandle<R>`).
enum ExportEvent {
    Progress(JobProgressDto),
    Notice(Notice),
}

type ExportEmitter = Arc<dyn Fn(ExportEvent) + Send + Sync>;

struct JobHandle {
    cancel: CancelToken,
}

struct Inner {
    documents: DocumentService,
    /// H-08: the live rack snapshot source (`EngineHandle::rack_model()`), read synchronously in
    /// `start_job` — the render never talks to the audio thread.
    engine: EngineHandle,
    registry: Arc<Registry>,
    /// SPEC-007 §4.1 (H-30): halves tile work while an export runs (`None` in tests that don't
    /// need it, mirroring `bake.rs`'s `Inner::spectro`).
    spectro: Option<Arc<SpectroService>>,
    emit: ExportEmitter,
    next_id: AtomicU32,
    jobs: Mutex<std::collections::HashMap<u32, JobHandle>>,
}

/// Cheaply cloneable (an `Arc` inside), managed as Tauri state (like [`DocumentService`]).
#[derive(Clone)]
pub struct ExportService(Arc<Inner>);

/// Builds the service for the running app (composition root: registers the built-in modules,
/// like `audio::start`).
pub fn start<R: Runtime>(
    app: AppHandle<R>,
    documents: DocumentService,
    engine: EngineHandle,
    spectro: Arc<SpectroService>,
) -> anyhow::Result<ExportService> {
    let registry = crate::plugins::registry()?;
    let emit: ExportEmitter = Arc::new(move |event| forward(&app, event));
    Ok(ExportService(Arc::new(Inner {
        documents,
        engine,
        registry,
        spectro: Some(spectro),
        emit,
        next_id: AtomicU32::new(1),
        jobs: Mutex::new(std::collections::HashMap::new()),
    })))
}

/// Runs on the job thread: map to DTOs and emit (mirrors `audio.rs::forward`).
fn forward<R: Runtime>(app: &AppHandle<R>, event: ExportEvent) {
    let result = match event {
        ExportEvent::Progress(dto) => emit_job_progress(app, dto),
        ExportEvent::Notice(notice) => emit_notice(app, notice),
    };
    if let Err(error) = result {
        tracing::warn!(%error, "emitting an export event failed");
    }
}

impl ExportService {
    /// `export_formats`: MP3 availability (ADR-007 §4, D-013). WAV/FLAC are always available.
    pub fn mp3_available(&self) -> bool {
        vox_io::mp3_available()
    }

    /// `export_start`: validates the range against the current document, snapshots the **live
    /// rack** (`EngineHandle::rack_model()`, H-08) synchronously — before the job thread spawns,
    /// so a rack edit made after this call returns never reaches the running export — then runs
    /// the job on its own thread. Returns the job id immediately; `job_progress` events (tagged
    /// with it) report progress and the final state.
    pub fn start_job(&self, request: ExportRequest) -> Result<u32, IpcError> {
        let source = self.0.documents.export_source()?;
        let (start, end) = resolve_range(request.range, source.len_samples)?;
        // `None` only if the engine's control thread is already gone (app shutdown) — falls back
        // to an empty rack rather than failing the export outright.
        let model = self.0.engine.rack_model().unwrap_or_default();
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
            .name("export-job".into())
            .spawn(move || {
                run_job(
                    inner,
                    job_id,
                    source,
                    model,
                    start,
                    end,
                    request.path,
                    request.format,
                    request.sample_rate_hz,
                    cancel,
                );
            })
            .map_err(|e| IpcError::internal(e.to_string()))?;
        Ok(job_id)
    }

    /// `export_cancel`: best-effort — the job notices at the next rack-render block (one
    /// `OFFLINE_BLOCK`, well under SPEC-010's ~100 ms convention this reuses) or phase boundary,
    /// and reports `JobState::Cancelled`. A no-op for an already-finished or unknown job id.
    pub fn cancel_job(&self, job_id: u32) {
        if let Some(job) = self.0.jobs.lock().unwrap().get(&job_id) {
            job.cancel.cancel();
        }
    }
}

fn resolve_range(range: Option<(u64, u64)>, len_samples: u64) -> Result<(u64, u64), IpcError> {
    match range {
        None => Ok((0, len_samples)),
        Some((start, end)) if start < end && end <= len_samples => Ok((start, end)),
        Some(_) => Err(IpcError::new(
            IpcErrorCode::InvalidArgument,
            "error.export.invalid_range",
        )),
    }
}

fn cancelled() -> IpcError {
    IpcError::new(IpcErrorCode::Cancelled, "error.cancelled")
}

fn check_cancelled(cancel: &CancelToken) -> Result<(), IpcError> {
    if cancel.is_cancelled() {
        Err(cancelled())
    } else {
        Ok(())
    }
}

fn project_read_error(err: vox_project::ProjectError) -> IpcError {
    IpcError::new(IpcErrorCode::Io, "error.export.read").with_param("message", err.to_string())
}

fn rack_error(err: vox_rack::offline::RenderError) -> IpcError {
    IpcError::new(IpcErrorCode::Internal, "error.export.rack")
        .with_param("message", err.to_string())
}

fn resample_error(err: vox_dsp::resample::ResampleError) -> IpcError {
    IpcError::new(IpcErrorCode::Internal, "error.export.resample")
        .with_param("message", err.to_string())
}

fn io_error(err: vox_io::IoError) -> IpcError {
    if let vox_io::IoError::Mp3Unavailable(_) = &err {
        return IpcError::new(
            IpcErrorCode::InvalidArgument,
            "error.export.mp3_unavailable",
        )
        .with_param("message", err.to_string());
    }
    IpcError::new(IpcErrorCode::Io, "error.export.io").with_param("message", err.to_string())
}

fn notice_from_error(err: &IpcError) -> Notice {
    let mut notice = Notice::toast(NoticeLevel::Error, err.key.clone());
    for (name, value) in &err.params {
        notice = notice.with_param(name.clone(), value.clone());
    }
    notice
}

#[allow(clippy::too_many_arguments)]
fn run_job(
    inner: Arc<Inner>,
    job_id: u32,
    source: ExportSource,
    model: RackModel,
    start: u64,
    end: u64,
    path: PathBuf,
    format: ExportFormat,
    target_rate_hz: u32,
    cancel: CancelToken,
) {
    // SPEC-007 §4.1 (H-30): tiles use at most half of the workers while the export runs (mirrors
    // bake.rs's own guard).
    let background = inner.spectro.as_ref().map(|s| s.begin_background_job());
    let emit = Arc::clone(&inner.emit);
    let result = run_pipeline(
        &inner.registry,
        &source,
        &model,
        start,
        end,
        &path,
        format,
        target_rate_hz,
        &cancel,
        |fraction| {
            emit(ExportEvent::Progress(JobProgressDto {
                job_id,
                kind: JobKind::Export,
                state: JobState::Running,
                fraction,
            }));
        },
    );
    drop(background);
    inner.jobs.lock().unwrap().remove(&job_id);

    let (state, fraction) = match &result {
        Ok(()) => (JobState::Done, 1.0),
        Err(err) if err.code == IpcErrorCode::Cancelled => (JobState::Cancelled, 0.0),
        Err(_) => (JobState::Failed, 0.0),
    };
    // The error notice goes out before the terminal `Failed` progress event, so anything that
    // sees `Failed` (the UI's job store, tests) already has the reason (bake.rs's `fail_job`
    // convention; H-30 made every job service follow it).
    if let (JobState::Failed, Err(err)) = (state, &result) {
        (inner.emit)(ExportEvent::Notice(notice_from_error(err)));
    }
    // H-50: the success notice goes out before the terminal `Done` progress event too (mirrors
    // the `Failed` ordering above, and H-30's convention generally), so anything that sees
    // `Done` already has it.
    if state == JobState::Done {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        (inner.emit)(ExportEvent::Notice(
            Notice::toast(NoticeLevel::Info, "notice.export.done").with_param("name", name),
        ));
    }
    (inner.emit)(ExportEvent::Progress(JobProgressDto {
        job_id,
        kind: JobKind::Export,
        state,
        fraction,
    }));
}

/// Renders `[start, end)` of `source`'s snapshot through `model` into memory (T-602: the render
/// path shared with Bake rack, `vox_engine::bake::render_document_range` — 4096-frame blocks,
/// latency trimmed, the range's context as pre-roll/post-roll, no tail). `progress` gets the
/// rendered fraction after every block; `cancel` stops it within one block. An empty range (an
/// empty document) renders nothing.
fn render_range(
    registry: &Registry,
    model: &RackModel,
    source: &ExportSource,
    start: u64,
    end: u64,
    cancel: &CancelToken,
    progress: impl FnMut(f32),
) -> Result<Vec<f32>, IpcError> {
    if start >= end {
        return Ok(Vec::new());
    }
    let range = vox_project::validate_range(start, end, source.len_samples)
        .map_err(|_| IpcError::new(IpcErrorCode::InvalidArgument, "error.export.invalid_range"))?;
    let doc = DocumentRange {
        store: &source.store,
        snapshot: &source.snapshot,
        sample_rate_hz: source.sample_rate_hz,
        range,
    };
    render_document_range_to_vec(registry, model, doc, cancel, progress).map_err(|e| match e {
        RangeRenderError::Cancelled => cancelled(),
        RangeRenderError::Render(vox_rack::offline::RenderError::SlotFailed { name, .. }) => {
            IpcError::new(IpcErrorCode::Internal, "error.export.slot_failed")
                .with_param("slot", name)
        }
        RangeRenderError::Render(other) => rack_error(other),
        RangeRenderError::Project(err) => project_read_error(err),
    })
}

/// Read → render (the live rack, H-08) → resample → encode, in that order (SPEC-005 §2.12's
/// "rack → resample → dither/quantize last → encode"; the encoders quantize internally). Reports
/// coarse progress: 0-70% the rack render (per-block, see [`render_range`]), 90% after
/// resampling, 100% after the encoder returns (encoders don't report progress mid-encode).
#[allow(clippy::too_many_arguments)]
fn run_pipeline(
    registry: &Registry,
    source: &ExportSource,
    model: &RackModel,
    start: u64,
    end: u64,
    path: &Path,
    format: ExportFormat,
    target_rate_hz: u32,
    cancel: &CancelToken,
    mut progress: impl FnMut(f32),
) -> Result<(), IpcError> {
    check_cancelled(cancel)?;
    let rendered = render_range(registry, model, source, start, end, cancel, |fraction| {
        progress(fraction * 0.7)
    })?;

    check_cancelled(cancel)?;
    let resampled = if target_rate_hz == source.sample_rate_hz {
        rendered
    } else {
        let input_f64: Vec<f64> = rendered.iter().map(|&s| f64::from(s)).collect();
        let out =
            vox_dsp::resample::resample_offline(&input_f64, source.sample_rate_hz, target_rate_hz)
                .map_err(resample_error)?;
        out.into_iter().map(|s| s as f32).collect()
    };
    progress(0.9);

    check_cancelled(cancel)?;
    match format {
        ExportFormat::Wav(bits) => {
            vox_io::write_wav(
                path,
                target_rate_hz,
                bits,
                vox_io::DitherMode::Tpdf,
                &resampled,
            )
            .map_err(io_error)?;
        }
        ExportFormat::Flac(bits) => {
            vox_io::write_flac(
                path,
                target_rate_hz,
                bits,
                vox_io::DitherMode::Tpdf,
                &resampled,
            )
            .map_err(io_error)?;
        }
        ExportFormat::Mp3(settings) => {
            vox_io::encode_mp3(path, target_rate_hz, &resampled, settings).map_err(io_error)?;
        }
    }
    progress(1.0);
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::process::Command;
    use std::time::Duration;

    use std::collections::BTreeMap;

    use vox_engine::backend::fake::{FakeBackend, FakeDevice, FakeDirection};
    use vox_engine::{Engine, EngineConfig, HostId, RackCommand};
    use vox_module_api::{ModuleRef, ModuleState, Version};
    use vox_modules::Gain;
    use vox_project::{ChunkStore, DocSnapshot, StoreOptions};
    use vox_rack::{RackModel, SlotModel};

    use super::*;

    fn tmp_dir(tag: &str) -> PathBuf {
        crate::test_util::tmp_dir(&format!("export-{tag}"))
    }

    fn source_from(dir: &Path, samples: &[f32], rate_hz: u32) -> ExportSource {
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
            suggested_name: Some("in".to_string()),
        }
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

    fn registry() -> Registry {
        Registry::with_factories(vox_modules::builtin_factories()).unwrap()
    }

    fn run(
        registry: &Registry,
        source: &ExportSource,
        path: &Path,
        format: ExportFormat,
        target_rate_hz: u32,
    ) -> Result<(), IpcError> {
        run_pipeline(
            registry,
            source,
            &RackModel::default(),
            0,
            source.len_samples,
            path,
            format,
            target_rate_hz,
            &CancelToken::new(),
            |_| {},
        )
    }

    /// A zero-crossing frequency estimate (linear interpolation between samples straddling zero),
    /// mirroring the local helper `vox_dsp::resample`'s own tests use for the same purpose.
    fn frequency_hz(x: &[f32], rate_hz: u32) -> f64 {
        let mut crossings = Vec::new();
        for i in 1..x.len() {
            if x[i - 1] < 0.0 && x[i] >= 0.0 {
                let frac = f64::from(-x[i - 1]) / f64::from(x[i] - x[i - 1]);
                crossings.push((i - 1) as f64 + frac);
            }
        }
        let n = crossings.len();
        assert!(n >= 2, "not enough zero crossings to estimate frequency");
        let periods = (n - 1) as f64;
        let span = crossings[n - 1] - crossings[0];
        f64::from(rate_hz) * periods / span
    }

    fn ffprobe_available() -> bool {
        Command::new("ffprobe")
            .arg("-version")
            .output()
            .is_ok_and(|o| o.status.success())
    }

    fn ffmpeg_decode_to_wav(input: &Path, output: &Path) -> bool {
        Command::new("ffmpeg")
            .args(["-y", "-i"])
            .arg(input)
            .arg(output)
            .output()
            .is_ok_and(|o| o.status.success())
    }

    /// Ticket AC: "Export a 1 kHz -20 dBFS sine through [Gain -6 dB]: WAV 24 -> peak
    /// -26.00 +/- 0.01 dBFS". `run_pipeline` itself always renders `RackModel::default()` (the
    /// "no rack UI yet" deviation noted at the top of this file), so this test exercises
    /// [`render_range`] directly with a Gain slot — the same render `run_pipeline` uses —
    /// followed by the real WAV encoder, to check the render+encode math end to end.
    #[test]
    fn wav24_export_through_gain_minus_6_db_hits_the_expected_peak() {
        let dir = tmp_dir("wav24-gain");
        let samples = vox_testkit::signal::sine(1000.0, -20.0, 0.5, 48_000).unwrap();
        let reg = registry();

        let model = gain_model(-6.0);
        let source = source_from(&dir, &samples, 48_000);
        let rendered = render_range(
            &reg,
            &model,
            &source,
            0,
            source.len_samples,
            &CancelToken::new(),
            |_| {},
        )
        .unwrap();
        let gained_path = dir.join("gained.wav");
        vox_io::write_wav(
            &gained_path,
            48_000,
            vox_io::BitDepth::Int24,
            vox_io::DitherMode::Tpdf,
            &rendered,
        )
        .unwrap();
        let (decoded, _info) = vox_testkit::wav::read_wav_file(&gained_path).unwrap();
        let peak = vox_testkit::measure::peak_dbfs(&decoded);
        assert!(
            (peak - (-26.0)).abs() <= 0.01,
            "peak {peak} dBFS, expected -26.00 +/- 0.01"
        );

        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Ticket AC: "FLAC decodes bit-exact to the 24-bit WAV".
    #[test]
    fn flac_export_decodes_bit_exact_to_the_wav_export() {
        let dir = tmp_dir("flac-bitexact");
        let samples = vox_testkit::signal::sine(997.0, -6.0, 0.3, 48_000).unwrap();
        let source = source_from(&dir, &samples, 48_000);
        let reg = registry();

        let wav_path = dir.join("out.wav");
        run(
            &reg,
            &source,
            &wav_path,
            ExportFormat::Wav(vox_io::BitDepth::Int24),
            48_000,
        )
        .unwrap();
        let flac_path = dir.join("out.flac");
        run(
            &reg,
            &source,
            &flac_path,
            ExportFormat::Flac(vox_io::FlacBitDepth::Int24),
            48_000,
        )
        .unwrap();

        let decoded_flac_wav = dir.join("decoded-from-flac.wav");
        let status = Command::new("flac")
            .args(["-d", "-f", "-o"])
            .arg(&decoded_flac_wav)
            .arg(&flac_path)
            .status()
            .expect("run flac -d");
        assert!(status.success(), "flac -d failed");

        let (wav_samples, wav_info) = vox_testkit::wav::read_wav_file(&wav_path).unwrap();
        let (flac_samples, flac_info) = vox_testkit::wav::read_wav_file(&decoded_flac_wav).unwrap();
        assert_eq!(wav_info.sample_rate, flac_info.sample_rate);
        assert_eq!(
            wav_samples, flac_samples,
            "FLAC must decode bit-exact to the WAV export"
        );

        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Ticket AC: "48 k -> 44.1 k keeps 1 kHz +/- 0.05% and level +/- 0.1 dB".
    #[test]
    fn resampled_export_keeps_frequency_and_level() {
        let dir = tmp_dir("resample");
        let samples = vox_testkit::signal::sine(1000.0, -6.0, 0.5, 48_000).unwrap();
        let source = source_from(&dir, &samples, 48_000);
        let reg = registry();

        let out_path = dir.join("out.wav");
        run(
            &reg,
            &source,
            &out_path,
            ExportFormat::Wav(vox_io::BitDepth::Float32),
            44_100,
        )
        .unwrap();

        let (decoded, info) = vox_testkit::wav::read_wav_file(&out_path).unwrap();
        assert_eq!(info.sample_rate, 44_100);
        // Trim the resampler's filter ringing at both ends before measuring.
        let trim = 200;
        let steady = &decoded[trim..decoded.len() - trim];
        let freq = frequency_hz(steady, 44_100);
        let rel_err_pct = (freq - 1000.0).abs() / 1000.0 * 100.0;
        assert!(
            rel_err_pct <= 0.05,
            "frequency {freq} Hz, {rel_err_pct}% off"
        );
        let peak = vox_testkit::measure::peak_dbfs(steady);
        assert!(
            (peak - (-6.0)).abs() <= 0.1,
            "peak {peak} dBFS, expected -6.0 +/- 0.1"
        );

        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Ticket AC: "MP3 (if libmp3lame present) decodes to the right rate/duration +/- 1 frame and
    /// level +/- 0.5 dB (report ffprobe: 44100 Hz, 192 kb/s CBR, mono)" — the ACX preset.
    #[test]
    fn acx_mp3_export_matches_the_acx_preset_and_decodes_correctly() {
        if !vox_io::mp3_available() {
            eprintln!("skipping: libmp3lame not installed");
            return;
        }
        let dir = tmp_dir("acx-mp3");
        let samples = vox_testkit::signal::sine(1000.0, -20.0, 2.0, 48_000).unwrap();
        let source = source_from(&dir, &samples, 48_000);
        let reg = registry();

        let mp3_path = dir.join("out.mp3");
        run(
            &reg,
            &source,
            &mp3_path,
            ExportFormat::Mp3(vox_io::Mp3Settings::ACX),
            44_100,
        )
        .unwrap();

        if ffprobe_available() {
            let output = Command::new("ffprobe")
                .args([
                    "-v",
                    "error",
                    "-select_streams",
                    "a:0",
                    "-show_entries",
                    "stream=sample_rate,channels,bit_rate",
                    "-of",
                    "default=noprint_wrappers=1",
                ])
                .arg(&mp3_path)
                .output()
                .expect("run ffprobe");
            let report = String::from_utf8_lossy(&output.stdout);
            println!("ffprobe report for the ACX export:\n{report}");
            assert!(report.contains("sample_rate=44100"), "{report}");
            assert!(report.contains("channels=1"), "{report}");

            let decoded_path = dir.join("decoded.wav");
            assert!(
                ffmpeg_decode_to_wav(&mp3_path, &decoded_path),
                "ffmpeg failed to decode the MP3 export"
            );
            let (decoded, info) = vox_testkit::wav::read_wav_file(&decoded_path).unwrap();
            assert_eq!(info.sample_rate, 44_100);

            let expected_duration_s = samples.len() as f64 / 48_000.0;
            let decoded_duration_s = decoded.len() as f64 / 44_100.0;
            // MP3 framing is 1152 samples/frame; allow +/- 1 frame at 44.1 kHz plus LAME's
            // documented encoder delay/padding either side.
            let frame_s = 1152.0 / 44_100.0;
            assert!(
                (decoded_duration_s - expected_duration_s).abs() <= frame_s * 3.0,
                "decoded duration {decoded_duration_s}s vs expected {expected_duration_s}s"
            );

            // Measure level over the steady middle of the tone, away from encoder/decoder edges.
            let mid = decoded.len() / 2;
            let half_window = (44_100.0 * 0.5) as usize;
            let start = mid.saturating_sub(half_window);
            let end = (mid + half_window).min(decoded.len());
            let level = vox_testkit::measure::rms_db(&decoded[start..end]);
            let expected_rms = -20.0 - 3.0; // sine RMS is ~3.01 dB below its peak
            assert!(
                (level - expected_rms).abs() <= 0.5,
                "level {level} dB, expected {expected_rms} +/- 0.5"
            );
        } else {
            eprintln!("skipping ffprobe/ffmpeg checks: not installed");
        }

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn resolve_range_validates_bounds() {
        assert_eq!(resolve_range(None, 100).unwrap(), (0, 100));
        assert_eq!(resolve_range(Some((10, 20)), 100).unwrap(), (10, 20));
        assert!(resolve_range(Some((20, 10)), 100).is_err());
        assert!(resolve_range(Some((0, 101)), 100).is_err());
    }

    #[test]
    fn cancellation_before_any_work_reports_cancelled_without_writing_a_file() {
        let dir = tmp_dir("cancel");
        let samples = vox_testkit::signal::sine(440.0, -6.0, 0.2, 48_000).unwrap();
        let source = source_from(&dir, &samples, 48_000);
        let reg = registry();
        let cancel = CancelToken::new();
        cancel.cancel();
        let out_path = dir.join("out.wav");
        let err = run_pipeline(
            &reg,
            &source,
            &RackModel::default(),
            0,
            source.len_samples,
            &out_path,
            ExportFormat::Wav(vox_io::BitDepth::Int16),
            48_000,
            &cancel,
            |_| {},
        )
        .unwrap_err();
        assert_eq!(err.code, IpcErrorCode::Cancelled);
        assert!(!out_path.exists());

        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Ticket AC: "a selection export has exactly the selection's length".
    #[test]
    fn selection_export_has_exactly_the_selections_length() {
        let dir = tmp_dir("selection-length");
        let samples = vox_testkit::signal::sine(440.0, -6.0, 1.0, 48_000).unwrap();
        let source = source_from(&dir, &samples, 48_000);
        let reg = registry();
        let out_path = dir.join("out.wav");
        let (start, end) = (10_000u64, 30_000u64);

        run_pipeline(
            &reg,
            &source,
            &RackModel::default(),
            start,
            end,
            &out_path,
            ExportFormat::Wav(vox_io::BitDepth::Float32),
            48_000,
            &CancelToken::new(),
            |_| {},
        )
        .unwrap();

        let (decoded, _info) = vox_testkit::wav::read_wav_file(&out_path).unwrap();
        assert_eq!(decoded.len() as u64, end - start);

        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A live engine (fake backend, real output device) with a Gain -6 dB slot: for tests that
    /// exercise `ExportService::start_job` against a real `EngineHandle::rack_model()`, retried
    /// like `nr_capture.rs`'s `live_engine_with_nr_slot` because the fake output opens
    /// asynchronously (its own device-poll thread).
    fn live_engine_with_gain_minus_6_db() -> (
        Engine,
        vox_engine::EngineHandle,
        vox_engine::backend::fake::FakeDriver,
    ) {
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
        let engine_registry =
            Arc::new(Registry::with_factories(vox_modules::builtin_factories()).unwrap());
        let engine = Engine::start(EngineConfig::new(Arc::new(fake), engine_registry)).unwrap();
        let handle = engine.handle();
        let mut opened = false;
        for _ in 0..500 {
            if handle
                .rack_command(RackCommand::Add {
                    module_id: Gain::ID.into(),
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
        handle
            .rack_command(RackCommand::SetParamText {
                index: 0,
                id: Gain::GAIN_DB,
                text: "-6.0".into(),
            })
            .unwrap();
        (engine, handle, driver)
    }

    /// Ticket ACs: "a live rack with Gain -6 dB -> exported peak is 6.00 +/- 0.01 dB lower than
    /// with an empty rack" and "the exported rack is the one live at export start (later rack
    /// edits don't change a running export)" — end to end through `ExportService::start_job`
    /// against a real `EngineHandle`: the Gain slot is removed right after the job starts, and
    /// the export must still show its -6 dB (H-08: `start_job` snapshots `rack_model()`
    /// synchronously, before the job thread spawns).
    #[test]
    fn export_renders_the_rack_live_at_start_ignoring_a_later_edit() {
        let dir = tmp_dir("live-rack-snapshot");
        let (_engine, handle, _driver) = live_engine_with_gain_minus_6_db();

        let wav_path = dir.join("in.wav");
        let samples = vox_testkit::signal::sine(1000.0, -20.0, 0.5, 48_000).unwrap();
        vox_testkit::wav::write_wav_file(
            &wav_path,
            &samples,
            1,
            48_000,
            vox_testkit::wav::BitDepth::Float32,
        )
        .unwrap();
        let documents = DocumentService::new(dir.join("sessions"), handle.clone());
        documents.open(&wav_path, false).unwrap();

        let done: Arc<Mutex<Option<JobState>>> = Arc::new(Mutex::new(None));
        let done_for_emit = Arc::clone(&done);
        let emit: ExportEmitter = Arc::new(move |event| {
            if let ExportEvent::Progress(dto) = event
                && !matches!(dto.state, JobState::Running)
            {
                *done_for_emit.lock().unwrap() = Some(dto.state);
            }
        });
        let inner = Arc::new(Inner {
            documents,
            engine: handle.clone(),
            registry: Arc::new(registry()),
            spectro: None,
            emit,
            next_id: AtomicU32::new(1),
            jobs: Mutex::new(std::collections::HashMap::new()),
        });
        let service = ExportService(inner);

        let out_path = dir.join("out.wav");
        service
            .start_job(ExportRequest {
                path: out_path.clone(),
                format: ExportFormat::Wav(vox_io::BitDepth::Int24),
                sample_rate_hz: 48_000,
                range: None,
            })
            .unwrap();

        // The "later rack edit": remove the Gain slot right after the job started. `start_job`
        // already captured `rack_model()` synchronously before this point, so it must not reach
        // the running export.
        handle
            .rack_command(RackCommand::Remove { index: 0 })
            .unwrap();

        let mut state = None;
        for _ in 0..500 {
            state = *done.lock().unwrap();
            if state.is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(state, Some(JobState::Done), "export job did not finish");

        let (decoded, _info) = vox_testkit::wav::read_wav_file(&out_path).unwrap();
        let peak = vox_testkit::measure::peak_dbfs(&decoded);
        assert!(
            (peak - (-26.0)).abs() <= 0.01,
            "peak {peak} dBFS, expected -26.00 +/- 0.01 (empty-rack peak would be -20.00)"
        );

        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// H-30 (SPEC-007 §4.1): "while a save, export or bake job runs, tile work uses at most half
    /// of the workers" — checked directly through `SpectroService::worker_cap_now` (a
    /// synchronous witness of the guard, not a race against real tile computation like
    /// `crates/engine/tests/spectro.rs`'s own test of the underlying mechanism): a large buffer
    /// resampled to a different rate keeps `run_job` busy long enough for the polling loop below
    /// to observe the halved cap without any fixed sleep to race against.
    #[test]
    fn export_holds_the_spectro_guard_for_the_whole_render() {
        let dir = tmp_dir("spectro-guard");
        let fake = FakeBackend::new(1);
        let engine_registry =
            Arc::new(Registry::with_factories(vox_modules::builtin_factories()).unwrap());
        let engine = Engine::start(EngineConfig::new(Arc::new(fake), engine_registry)).unwrap();
        let documents = DocumentService::new(dir.join("sessions"), engine.handle());

        let samples = vox_testkit::signal::pink_noise(8, -20.0, 60.0, 48_000).unwrap();
        let len = samples.len() as u64;
        let source = source_from(&dir, &samples, 48_000);
        let spectro = Arc::new(vox_engine::spectro::SpectroService::new(
            vox_engine::spectro::SpectroConfig {
                workers: 4,
                cache_cap_bytes: None,
            },
        ));
        let inner = Arc::new(Inner {
            documents,
            engine: engine.handle(),
            registry: Arc::new(registry()),
            spectro: Some(Arc::clone(&spectro)),
            emit: Arc::new(|_| {}),
            next_id: AtomicU32::new(1),
            jobs: Mutex::new(std::collections::HashMap::new()),
        });

        let out_path = dir.join("out.wav");
        let done = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let done_for_job = Arc::clone(&done);
        std::thread::Builder::new()
            .name("export-guard-test-job".into())
            .spawn(move || {
                run_job(
                    inner,
                    1,
                    source,
                    gain_model(-6.0),
                    0,
                    len,
                    out_path,
                    ExportFormat::Wav(vox_io::BitDepth::Int16),
                    // A real resample (44.1 kHz) is real DSP work on the whole buffer, on top of
                    // the render — comfortably slower than a bit-identical WAV copy.
                    44_100,
                    CancelToken::new(),
                );
                done_for_job.store(true, std::sync::atomic::Ordering::SeqCst);
            })
            .unwrap();

        let mut saw_halved = false;
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while !done.load(std::sync::atomic::Ordering::SeqCst) {
            if spectro.worker_cap_now() == 2 {
                saw_halved = true;
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "the export job never finished within 10s"
            );
            std::thread::sleep(Duration::from_micros(200));
        }
        // Let the job finish either way, then confirm the guard released the cap.
        while !done.load(std::sync::atomic::Ordering::SeqCst) {
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(
            saw_halved,
            "never observed the halved tile-worker cap while the export ran"
        );
        assert_eq!(
            spectro.worker_cap_now(),
            4,
            "the cap is restored once the export finishes"
        );

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[derive(Debug, Clone, PartialEq)]
    enum SuccessTestEvent {
        Progress(JobState),
        Notice(String),
    }

    fn wait_for_finish(events: &Mutex<Vec<SuccessTestEvent>>) -> JobState {
        for _ in 0..500 {
            if let Some(state) = events.lock().unwrap().iter().find_map(|e| match e {
                SuccessTestEvent::Progress(state) if *state != JobState::Running => Some(*state),
                _ => None,
            }) {
                return state;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("the export job never reported a terminal state within 5s");
    }

    /// H-50: `notice.export.done` used to go out *after* the terminal `Done` progress event, so
    /// under load a test (or the UI) that reacted to `Done` could miss it — the same bug H-30
    /// fixed for the `Failed` path. `run_job` on its own thread (like the real job), polled with
    /// `wait_for_finish` rather than raced: the notice must already be in `events` the moment
    /// `Done` is first observed, with no further waiting.
    #[test]
    fn a_successful_job_posts_the_done_notice_before_the_terminal_done_progress_event() {
        let dir = tmp_dir("success-order");
        let fake = FakeBackend::new(1);
        let engine_registry =
            Arc::new(Registry::with_factories(vox_modules::builtin_factories()).unwrap());
        let engine = Engine::start(EngineConfig::new(Arc::new(fake), engine_registry)).unwrap();
        let documents = DocumentService::new(dir.join("sessions"), engine.handle());

        let samples = vox_testkit::signal::sine(440.0, -12.0, 0.2, 48_000).unwrap();
        let len = samples.len() as u64;
        let source = source_from(&dir, &samples, 48_000);

        let events: Arc<Mutex<Vec<SuccessTestEvent>>> = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&events);
        let emit: ExportEmitter = Arc::new(move |event| {
            let e = match event {
                ExportEvent::Progress(dto) => SuccessTestEvent::Progress(dto.state),
                ExportEvent::Notice(n) => SuccessTestEvent::Notice(n.key),
            };
            sink.lock().unwrap().push(e);
        });
        let inner = Arc::new(Inner {
            documents,
            engine: engine.handle(),
            registry: Arc::new(registry()),
            spectro: None,
            emit,
            next_id: AtomicU32::new(1),
            jobs: Mutex::new(std::collections::HashMap::new()),
        });

        let out_path = dir.join("out.wav");
        std::thread::Builder::new()
            .name("export-success-order-job".into())
            .spawn(move || {
                run_job(
                    inner,
                    1,
                    source,
                    RackModel::default(),
                    0,
                    len,
                    out_path,
                    ExportFormat::Wav(vox_io::BitDepth::Int24),
                    48_000,
                    CancelToken::new(),
                );
            })
            .unwrap();

        assert_eq!(wait_for_finish(&events), JobState::Done);
        let got = events.lock().unwrap().clone();
        assert!(
            got.iter()
                .any(|e| matches!(e, SuccessTestEvent::Notice(key) if key == "notice.export.done")),
            "the done notice must already be present once Done is observed: {got:?}"
        );

        std::fs::remove_dir_all(&dir).unwrap();
    }
}

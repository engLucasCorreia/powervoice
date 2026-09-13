//! Export job (S4-04, H-08): renders the **live rack** (`EngineHandle::rack_model()`, snapshotted
//! synchronously in [`ExportService::start_job`] before the job thread spawns — later rack edits
//! never affect a running export) over the whole document or a sample range through
//! `vox_rack::offline` (SPEC-012 §2.8, time-aligned), converts sample rate
//! (`vox_dsp::resample::resample_offline`, f64) and bit depth/format via S4-02's encoders
//! (`vox_io`), and writes atomically. Runs as a cancellable job on its own thread, reporting
//! `job_progress` (ADR-003) at each rack-render block plus phase boundaries; never touches the
//! document (D-019 — only Save/Save As write it).
//!
//! **H-08:** the rack panel (S3-01) and selection (S2-01) have both landed, so the "rack is
//! always empty" / "whole file only" deviations this module used to carry are resolved: the
//! dialog's Whole file/Selection choice sends `range` through unchanged (it was already plumbed
//! end to end), and SPEC-014's "Output noise only" pre-export confirmation is UI-side
//! (`ui/src/lib/export/export.svelte.ts`), reading the same live `RackStateDto` the rack panel
//! shows.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use tauri::{AppHandle, Runtime};
use vox_engine::EngineHandle;
use vox_project::{CHUNK_SAMPLES, SnapshotReader};
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
    cancel: Arc<AtomicBool>,
}

struct Inner {
    documents: DocumentService,
    /// H-08: the live rack snapshot source (`EngineHandle::rack_model()`), read synchronously in
    /// `start_job` — the render never talks to the audio thread.
    engine: EngineHandle,
    registry: Arc<Registry>,
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
) -> anyhow::Result<ExportService> {
    let registry = Arc::new(Registry::with_factories(vox_modules::builtin_factories())?);
    let emit: ExportEmitter = Arc::new(move |event| forward(&app, event));
    Ok(ExportService(Arc::new(Inner {
        documents,
        engine,
        registry,
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
        let cancel = Arc::new(AtomicBool::new(false));
        self.0.jobs.lock().unwrap().insert(
            job_id,
            JobHandle {
                cancel: Arc::clone(&cancel),
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
            job.cancel.store(true, Ordering::Relaxed);
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

fn check_cancelled(cancel: &AtomicBool) -> Result<(), IpcError> {
    if cancel.load(Ordering::Relaxed) {
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
    cancel: Arc<AtomicBool>,
) {
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
    inner.jobs.lock().unwrap().remove(&job_id);

    let (state, fraction) = match &result {
        Ok(()) => (JobState::Done, 1.0),
        Err(err) if err.code == IpcErrorCode::Cancelled => (JobState::Cancelled, 0.0),
        Err(_) => (JobState::Failed, 0.0),
    };
    (inner.emit)(ExportEvent::Progress(JobProgressDto {
        job_id,
        kind: JobKind::Export,
        state,
        fraction,
    }));
    match (state, result) {
        (JobState::Done, _) => {
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            (inner.emit)(ExportEvent::Notice(
                Notice::toast(NoticeLevel::Info, "notice.export.done").with_param("name", name),
            ));
        }
        (JobState::Failed, Err(err)) => {
            (inner.emit)(ExportEvent::Notice(notice_from_error(&err)));
        }
        _ => {}
    }
}

/// Reads `[start, end)` of `source`'s current snapshot into memory (the same "whole buffer in
/// RAM" approach `vox_project::save_snapshot_wav` uses today — H-02 backlog item to stream it).
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

/// The same block loop as `vox_rack::offline::render` (4096-frame blocks, latency trim), reused
/// here (not modified in `vox_rack`, out of this ticket's scope) instead of called through it, so
/// export can report progress and check `cancel` every block. No automation: export renders the
/// rack's current static parameters (SPEC-012 §2.8's "time-aligned" render, no automation events
/// in this ticket's scope).
fn render_with_progress(
    registry: &Registry,
    model: &RackModel,
    sample_rate: f64,
    input: &[f32],
    cancel: &AtomicBool,
    mut progress: impl FnMut(f32),
) -> Result<Vec<f32>, IpcError> {
    // SAFETY-equivalent note: no `unsafe` here; the FTZ/DAZ guard just sets thread-local FPU mode
    // for the render (ADR-002 §2, `vox_dsp::fp`), like every other offline render.
    let _fp = vox_dsp::fp::DenormalGuard::new();
    let mut chain =
        vox_rack::offline::build_chain(registry, model, sample_rate).map_err(rack_error)?;
    let latency = chain.latency_samples() as usize;
    let total = input.len() + latency;
    if total == 0 {
        chain.deactivate();
        return Ok(Vec::new());
    }
    let mut out = vec![0.0f32; total];
    let block = vox_rack::OFFLINE_BLOCK as usize;
    let mut inbuf = vec![0.0f32; block];
    let mut pos = 0usize;
    while pos < total {
        if cancel.load(Ordering::Relaxed) {
            chain.deactivate();
            return Err(cancelled());
        }
        let n = block.min(total - pos);
        let avail = input.len().saturating_sub(pos).min(n);
        inbuf[..avail].copy_from_slice(&input[pos..pos + avail]);
        inbuf[avail..n].fill(0.0);
        chain.process(
            vox_rack::Transport {
                playing: true,
                position_samples: Some(pos as u64),
            },
            &inbuf[..n],
            &mut out[pos..pos + n],
        );
        if let Some((index, _reason)) = chain.take_failure() {
            let name = chain.slot_name(index).unwrap_or_default().to_owned();
            chain.deactivate();
            return Err(
                IpcError::new(IpcErrorCode::Internal, "error.export.slot_failed")
                    .with_param("slot", name),
            );
        }
        chain.drain_events(|_| {});
        pos += n;
        progress(pos as f32 / total as f32);
    }
    chain.deactivate();
    out.drain(..latency);
    out.truncate(input.len());
    Ok(out)
}

/// Read → render (the live rack, H-08) → resample → encode, in that order (SPEC-005 §2.12's
/// "rack → resample → dither/quantize last → encode"; the encoders quantize internally). Reports
/// coarse progress: 0-70% the rack render (per-block, see [`render_with_progress`]), 90% after
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
    cancel: &AtomicBool,
    mut progress: impl FnMut(f32),
) -> Result<(), IpcError> {
    check_cancelled(cancel)?;
    let samples = read_range(source, start, end)?;

    check_cancelled(cancel)?;
    let rendered = render_with_progress(
        registry,
        model,
        f64::from(source.sample_rate_hz),
        &samples,
        cancel,
        |fraction| progress(fraction * 0.7),
    )?;

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
            vox_io::write_wav(path, target_rate_hz, bits, &resampled).map_err(io_error)?;
        }
        ExportFormat::Flac(bits) => {
            vox_io::write_flac(path, target_rate_hz, bits, &resampled).map_err(io_error)?;
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
    use std::sync::atomic::AtomicBool;
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
        let dir = std::env::temp_dir().join(format!(
            "powervoice-app-export-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
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
            &AtomicBool::new(false),
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
    /// [`render_with_progress`] directly with a Gain slot — the same block loop `run_pipeline`
    /// uses — followed by the real WAV encoder, to check the render+encode math end to end.
    #[test]
    fn wav24_export_through_gain_minus_6_db_hits_the_expected_peak() {
        let dir = tmp_dir("wav24-gain");
        let samples = vox_testkit::signal::sine(1000.0, -20.0, 0.5, 48_000).unwrap();
        let reg = registry();

        let model = gain_model(-6.0);
        let rendered = render_with_progress(
            &reg,
            &model,
            48_000.0,
            &samples,
            &AtomicBool::new(false),
            |_| {},
        )
        .unwrap();
        let gained_path = dir.join("gained.wav");
        vox_io::write_wav(&gained_path, 48_000, vox_io::BitDepth::Int24, &rendered).unwrap();
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
        let cancel = AtomicBool::new(true);
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
            &AtomicBool::new(false),
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
        documents.open(&wav_path).unwrap();

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
}

use std::io::Write as _;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use tauri::ipc::{Channel, InvokeResponseBody, Response};

use crate::ipc::{IpcError, IpcErrorCode};
use crate::spike::binary;
use crate::spike::dto::{SpikeEnv, WaveformMeta};

fn internal_error(message: impl Into<String>) -> IpcError {
    let mut params = std::collections::HashMap::new();
    params.insert("message".to_string(), message.into());
    IpcError {
        code: IpcErrorCode::Internal,
        key: "spike.internal_error".to_string(),
        params,
    }
}

fn env_flag(name: &str) -> bool {
    std::env::var(name).map(|v| v == "1").unwrap_or(false)
}

/// Tells the UI whether (and how) to run the spike. Only exists when built with
/// `--features spike`, which is exactly how the frontend detects a spike build (see
/// `ui/src/spike/detect.ts`).
#[tauri::command]
pub async fn spike_env() -> Result<SpikeEnv, IpcError> {
    Ok(SpikeEnv {
        auto_run: env_flag("POWERVOICE_SPIKE"),
        exit_after: env_flag("POWERVOICE_SPIKE_EXIT"),
        webkit_dmabuf_disabled: env_flag("WEBKIT_DISABLE_DMABUF_RENDERER"),
    })
}

// --- Waveform peaks (scope item 1) -----------------------------------------------------------

const SAMPLE_RATE_HZ: u32 = 48_000;
const DURATION_S: u64 = 60 * 60; // 60-minute synthetic file
const TOTAL_SAMPLES: u64 = SAMPLE_RATE_HZ as u64 * DURATION_S; // 172_800_000
const SPP: u32 = 512;
/// Peaks are approximated from evenly spaced probes per bucket rather than a full per-sample
/// scan: this is synthetic bench data (no real document), and a full 172.8M-sample scan bought
/// no fidelity worth the extra generation time, especially in debug builds.
const PROBES_PER_BUCKET: u32 = 32;

fn synth_sample(sample_index: u64) -> f32 {
    let t = sample_index as f64 / f64::from(SAMPLE_RATE_HZ);
    let envelope = 0.5 + 0.5 * (t * 0.05).sin();
    let tone = (t * 220.0 * std::f64::consts::TAU).sin() * 0.6
        + (t * 3300.0 * std::f64::consts::TAU).sin() * 0.15;
    let noise =
        (sample_index.wrapping_mul(2_654_435_761) >> 8) as f64 / f64::from(u32::MAX) * 2.0 - 1.0;
    ((tone + noise * 0.05) * envelope) as f32
}

fn generate_waveform_peaks() -> (Vec<(f32, f32)>, Duration) {
    let start = Instant::now();
    let bucket_count = (TOTAL_SAMPLES / u64::from(SPP)) as u32;
    let step = (SPP / PROBES_PER_BUCKET).max(1);
    let mut peaks = Vec::with_capacity(bucket_count as usize);
    for bucket in 0..bucket_count {
        let bucket_start = u64::from(bucket) * u64::from(SPP);
        let mut min = f32::INFINITY;
        let mut max = f32::NEG_INFINITY;
        let mut i = 0u32;
        while i < SPP {
            let v = synth_sample(bucket_start + u64::from(i));
            min = min.min(v);
            max = max.max(v);
            i += step;
        }
        peaks.push((min, max));
    }
    (peaks, start.elapsed())
}

/// Waveform peaks for a synthetic 60-minute 48 kHz file, generated in Rust and delivered over a
/// `Channel` as a binary `SPWV` frame (ADR-003 §2 conventions). The JS side slices/decimates this
/// single buffer to simulate the zoom sweep, exactly like the real waveform view will pick a
/// coarser/finer peak level and reduce buckets per pixel (ADR-003).
#[tauri::command]
pub async fn spike_waveform_peaks(channel: Channel) -> Result<WaveformMeta, IpcError> {
    let (peaks, generation) = generate_waveform_peaks();
    let count = peaks.len() as u32;
    let frame = binary::waveform_frame(&peaks, SPP);
    let frame_bytes = frame.len() as u32;
    channel
        .send(InvokeResponseBody::Raw(frame))
        .map_err(|e| internal_error(e.to_string()))?;
    Ok(WaveformMeta {
        total_samples: TOTAL_SAMPLES,
        sample_rate_hz: SAMPLE_RATE_HZ,
        spp: SPP,
        count,
        generation_ms: generation.as_secs_f64() * 1000.0,
        frame_bytes,
    })
}

// --- Spectrogram texture (scope item 2) ------------------------------------------------------

const SPECTRO_WIDTH: u32 = 1024;
const SPECTRO_HEIGHT: u32 = 512;

fn generate_spectrogram_pixels() -> (Vec<u8>, Duration) {
    let start = Instant::now();
    let mut pixels = Vec::with_capacity((SPECTRO_WIDTH * SPECTRO_HEIGHT) as usize);
    for y in 0..SPECTRO_HEIGHT {
        for x in 0..SPECTRO_WIDTH {
            let freq = f64::from(y) / f64::from(SPECTRO_HEIGHT);
            let time = f64::from(x) / f64::from(SPECTRO_WIDTH);
            let bands = (time * 40.0 + freq * 6.0).sin() * 0.5 + 0.5;
            let formant = (-((freq - 0.3).powi(2)) * 40.0).exp();
            let v = (bands * 0.6 + formant * 0.4) * 255.0;
            pixels.push(v.clamp(0.0, 255.0) as u8);
        }
    }
    (pixels, start.elapsed())
}

/// A synthetic 1024x512 u8 magnitude "spectrogram" texture, delivered via `ipc::Response` (raw
/// bytes, no JSON) as an `SPST` frame. The JS renderer uploads it once and then scrolls/redraws
/// it every animation frame to measure WebGL2 vs Canvas2D update cost (the ticket's rendering
/// question is about draw/update cost, not per-frame IPC, so this is a single fetch).
#[tauri::command]
pub async fn spike_spectrogram_texture() -> Result<Response, IpcError> {
    let (pixels, generation) = generate_spectrogram_pixels();
    let frame_bytes = (24 + pixels.len()) as u32;
    let frame = binary::spectrogram_frame(&pixels, SPECTRO_WIDTH, SPECTRO_HEIGHT);
    // Stash meta as a side value isn't possible on a raw Response; the JS side already knows the
    // requested width/height, and `generation_ms` is logged (not measured) here for interest.
    tracing_stub_log(&format!(
        "spike_spectrogram_texture: {frame_bytes} bytes in {:.2}ms",
        generation.as_secs_f64() * 1000.0
    ));
    Ok(Response::new(frame))
}

/// `src-tauri` has no logging setup yet (T-104); this is a `stderr` breadcrumb for the person
/// running the spike, not a real logging facility.
fn tracing_stub_log(message: &str) {
    eprintln!("[spike] {message}");
}

// --- IPC throughput (scope item 3) -----------------------------------------------------------

const THROUGHPUT_BYTES: usize = 10 * 1024 * 1024; // 10 MB

fn fill_deterministic(len: usize) -> Vec<u8> {
    (0..len).map(|i| (i % 256) as u8).collect()
}

/// 10 MB via `ipc::Response`, for the ADR-003 throughput comparison against `spike_ipc_channel_10mb`.
#[tauri::command]
pub async fn spike_ipc_response_10mb() -> Result<Response, IpcError> {
    Ok(Response::new(fill_deterministic(THROUGHPUT_BYTES)))
}

/// 10 MB via a `Channel`, for the ADR-003 throughput comparison against `spike_ipc_response_10mb`.
#[tauri::command]
pub async fn spike_ipc_channel_10mb(channel: Channel) -> Result<(), IpcError> {
    channel
        .send(InvokeResponseBody::Raw(fill_deterministic(
            THROUGHPUT_BYTES,
        )))
        .map_err(|e| internal_error(e.to_string()))?;
    Ok(())
}

// --- Telemetry channel cost (MEMORY.md follow-up (b)) ----------------------------------------

/// Streams `VXTM`-shaped 72-byte frames over `channel` at `hz` for `duration_ms`, then stops.
/// Returns immediately; the JS side already knows `duration_ms` so it can stop listening and
/// tally received frames / dropped `requestAnimationFrame`s over that same window without needing
/// a completion signal from Rust.
#[tauri::command]
pub async fn spike_telemetry_run(
    channel: Channel,
    hz: u32,
    duration_ms: u32,
) -> Result<(), IpcError> {
    if hz == 0 {
        return Err(internal_error("hz must be > 0"));
    }
    let period = Duration::from_secs_f64(1.0 / f64::from(hz));
    let duration = Duration::from_millis(u64::from(duration_ms));
    std::thread::spawn(move || {
        let start = Instant::now();
        let mut seq = 0u32;
        while start.elapsed() < duration {
            let frame = binary::telemetry_frame(seq);
            if channel
                .send(InvokeResponseBody::Raw(frame.to_vec()))
                .is_err()
            {
                break;
            }
            seq = seq.wrapping_add(1);
            std::thread::sleep(period);
        }
    });
    Ok(())
}

// --- Results + lifecycle ----------------------------------------------------------------------

/// Directory the spike writes results into, resolved from the crate's manifest directory (stable
/// regardless of the OS working directory the Tauri binary happens to be launched with) rather
/// than `std::env::current_dir()`.
fn bench_results_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("src-tauri has a parent (the repo root)")
        .join("bench-results")
}

/// Writes `results_json` (already assembled by the JS harness) to
/// `bench-results/spike-<unix_ms>.json` and returns the path written, so the automated runner can
/// print/read it back. `bench-results/` is gitignored (CLAUDE.md / ticket).
#[tauri::command]
pub async fn spike_write_results(results_json: String) -> Result<String, IpcError> {
    let dir = bench_results_dir();
    std::fs::create_dir_all(&dir).map_err(|e| internal_error(e.to_string()))?;
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| internal_error(e.to_string()))?
        .as_millis();
    let path = dir.join(format!("spike-{millis}.json"));
    let mut file = std::fs::File::create(&path).map_err(|e| internal_error(e.to_string()))?;
    file.write_all(results_json.as_bytes())
        .map_err(|e| internal_error(e.to_string()))?;
    Ok(path.to_string_lossy().into_owned())
}

/// Closes the app. Used only when `POWERVOICE_SPIKE_EXIT=1` (automated runs) after results are
/// written; otherwise the window is left open for the owner's manual input checks.
///
/// `AppHandle<R>` (unlike `Channel<T>`, which has no `Runtime` type parameter) must stay generic
/// over `R` here: `ipc_commands!`/`invoke_handler` are themselves generic over `R: tauri::Runtime`
/// so this command can be monomorphized for whatever runtime Tauri actually instantiates.
#[tauri::command]
pub async fn spike_exit<R: tauri::Runtime>(app: tauri::AppHandle<R>) -> Result<(), IpcError> {
    app.exit(0);
    Ok(())
}

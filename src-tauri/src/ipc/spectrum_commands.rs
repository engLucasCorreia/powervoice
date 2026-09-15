//! Long-term average spectrum commands (H-42, SPEC-007 §8.5): thin wrappers over
//! [`SpectrumService`] (CLAUDE.md: no business logic here). `spectrum_analyze_start` returns at
//! once; progress arrives as `job_progress`, the diagnostics as `spectrum_report`, and each
//! curve on request as a binary `VXLT` frame (`spectrum_analyze_curve`). `spectrum_export_csv`
//! writes the Spectrum Inspector's CSV export to the path the save dialog picked.

use std::path::PathBuf;

use tauri::State;
use tauri::ipc::Response;

use crate::ipc::error::IpcError;
use crate::ipc::spectrum_dto::{SpectrumAnalyzeRequestDto, SpectrumAnalyzeStartedDto};
use crate::spectrum::{SpectrumRequest, SpectrumService, write_csv};

/// Starts a long-term average job; the returned `job_id` tags its `job_progress`/
/// `spectrum_report` events.
#[tauri::command]
pub async fn spectrum_analyze_start(
    spectrum: State<'_, SpectrumService>,
    request: SpectrumAnalyzeRequestDto,
) -> Result<SpectrumAnalyzeStartedDto, IpcError> {
    let range = match (request.start_sample, request.end_sample) {
        (Some(start), Some(end)) => Some((start, end)),
        _ => None,
    };
    let job_id = spectrum.start_job(SpectrumRequest {
        range,
        sources: request.sources.into_iter().map(Into::into).collect(),
        fft_size: request.fft_size,
        window: request.window.into(),
    })?;
    Ok(SpectrumAnalyzeStartedDto { job_id })
}

/// Cancels a running job (best-effort; `job_progress` reports `cancelled`).
#[tauri::command]
pub async fn spectrum_analyze_cancel(
    spectrum: State<'_, SpectrumService>,
    job_id: u32,
) -> Result<(), IpcError> {
    spectrum.cancel_job(job_id);
    Ok(())
}

/// Result `index` of a finished job, as a binary `VXLT` frame.
#[tauri::command]
pub async fn spectrum_analyze_curve(
    spectrum: State<'_, SpectrumService>,
    job_id: u32,
    index: u32,
) -> Result<Response, IpcError> {
    Ok(Response::new(spectrum.curve(job_id, index)?))
}

/// Writes the Inspector's CSV export; returns the path written (`.csv` added when missing).
#[tauri::command]
pub async fn spectrum_export_csv(path: String, contents: String) -> Result<String, IpcError> {
    tauri::async_runtime::spawn_blocking(move || {
        write_csv(&PathBuf::from(path), &contents).map(|p| p.to_string_lossy().into_owned())
    })
    .await
    .map_err(|e| IpcError::internal(e.to_string()))?
}

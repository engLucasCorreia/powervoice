//! Document commands (S1-03): open/save WAV, peaks. Thin: each forwards to [`DocumentService`]
//! and maps the result to a DTO or, for `peaks_get`, raw `VXPK` bytes over `ipc::Response`
//! (ADR-003 §2; CLAUDE.md — bulk IPC data is binary, never JSON float arrays).

use tauri::ipc::Response;
use tauri::{AppHandle, Emitter, Runtime, State};
use vox_project::{PEAKS_RAW_SPP, VxpkHeader, encode_vxpk};

use crate::document::DocumentService;
use crate::ipc::document_dto::{DocumentDto, PeaksRequestDto};
use crate::ipc::error::IpcError;
use crate::ipc::events::EventName;
use crate::settings::BitDepth;

/// ADR-003 §2's request cap: 65 536 buckets, or 1 Mi samples in `RAW` mode (4 MiB either way).
const MAX_BUCKETS: u32 = 65_536;
const MAX_RAW_SAMPLES: u32 = 1 << 20;

fn emit_document_changed<R: Runtime>(app: &AppHandle<R>, info: &DocumentDto) {
    if let Err(error) = app.emit(EventName::document_changed.as_str(), info.clone()) {
        tracing::warn!(%error, "emitting document_changed failed");
    }
}

async fn run_blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, IpcError> + Send + 'static,
) -> Result<T, IpcError> {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| IpcError::internal(e.to_string()))?
}

/// Opens `path` as the document (SPEC-005 §2.3: only WAV 16/24-bit int and 32-bit float are
/// accepted, S1-02 scope; other variants are `error.open.unsupported_format`). Replaces whatever
/// was open — the frontend runs the unsaved-changes prompt (simple version, SPEC-004 §2.8) first.
#[tauri::command]
pub async fn document_open<R: Runtime>(
    app: AppHandle<R>,
    doc: State<'_, DocumentService>,
    path: String,
) -> Result<DocumentDto, IpcError> {
    let doc = (*doc).clone();
    let info: DocumentDto = run_blocking(move || doc.open(std::path::Path::new(&path)))
        .await?
        .into();
    emit_document_changed(&app, &info);
    Ok(info)
}

/// Saves the current revision back to its bound path and format (SPEC-005 §2.7). No rack
/// rendering (D-019): this writes exactly the document's audio.
#[tauri::command]
pub async fn document_save<R: Runtime>(
    app: AppHandle<R>,
    doc: State<'_, DocumentService>,
) -> Result<DocumentDto, IpcError> {
    let doc = (*doc).clone();
    let info: DocumentDto = run_blocking(move || doc.save()).await?.into();
    emit_document_changed(&app, &info);
    Ok(info)
}

/// Saves the current revision to `path` at `bits`, then binds the document to it.
#[tauri::command]
pub async fn document_save_as<R: Runtime>(
    app: AppHandle<R>,
    doc: State<'_, DocumentService>,
    path: String,
    bits: BitDepth,
) -> Result<DocumentDto, IpcError> {
    let doc = (*doc).clone();
    let info: DocumentDto = run_blocking(move || doc.save_as(std::path::Path::new(&path), bits))
        .await?
        .into();
    emit_document_changed(&app, &info);
    Ok(info)
}

/// `count` buckets of `(min, max)` (or raw samples below `PEAKS_RAW_SPP`'s pyramid floor) as a
/// binary `VXPK` frame (ADR-003 §2, S1-02 `peaks_query::peaks` + `encode_vxpk`).
#[tauri::command]
pub async fn peaks_get(
    doc: State<'_, DocumentService>,
    request: PeaksRequestDto,
) -> Result<Response, IpcError> {
    let doc = (*doc).clone();
    let cap = if request.spp <= PEAKS_RAW_SPP {
        MAX_RAW_SAMPLES
    } else {
        MAX_BUCKETS
    };
    let count = request.count.min(cap);
    let spp = request.spp;
    let start = request.start_sample;
    let result = run_blocking(move || doc.peaks(spp, start, count)).await?;
    let header = VxpkHeader {
        request_id: request.request_id,
        audio_rev: result.audio_rev,
        start_sample: request.start_sample,
        samples_per_bucket: request.spp,
        sample_rate_hz: result.sample_rate_hz,
        partial: false,
    };
    Ok(Response::new(encode_vxpk(&header, &result.buckets)))
}

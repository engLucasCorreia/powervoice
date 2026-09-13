//! Document commands (S1-03): open/save WAV, peaks. Thin: each forwards to [`DocumentService`]
//! and maps the result to a DTO or, for `peaks_get`, raw `VXPK` bytes over `ipc::Response`
//! (ADR-003 §2; CLAUDE.md — bulk IPC data is binary, never JSON float arrays).

use tauri::ipc::Response;
use tauri::{AppHandle, Emitter, Runtime, State};
use vox_project::{PEAKS_RAW_SPP, VxpkHeader, encode_vxpk};

use crate::document::{DocumentService, PasteTarget};
use crate::ipc::document_dto::{
    ClipboardChangedDto, DocumentDto, EditResultDto, EditTargetDto, HistoryStateDto, MarkerDto,
    MarkerRangeKindDto, PeaksRequestDto,
};
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

/// S2-01: the Edit menu's Undo/Redo state, after every edit_*/history_undo/history_redo command.
fn emit_history_state<R: Runtime>(app: &AppHandle<R>, doc: &DocumentService) {
    let state: HistoryStateDto = doc.history_state().into();
    if let Err(error) = app.emit(EventName::history_state.as_str(), state) {
        tracing::warn!(%error, "emitting history_state failed");
    }
}

/// S2-01: the clipboard's length/rate, after `edit_cut`/`edit_copy` (SPEC-008 §4.3).
fn emit_clipboard_changed<R: Runtime>(app: &AppHandle<R>, doc: &DocumentService) {
    let state: ClipboardChangedDto = doc.clipboard_info().into();
    if let Err(error) = app.emit(EventName::clipboard_changed.as_str(), state) {
        tracing::warn!(%error, "emitting clipboard_changed failed");
    }
}

/// Common tail of every S2-01 command: `document_changed` (`audio_rev`/`len_samples`/dirty for
/// the waveform and title bar) and `history_state` (the Edit menu).
/// `pub(crate)`: also used by `loudness_commands::edit_normalize_lufs` (S4-01), which is the same
/// shape of command as every other S2-01/S2-02 edit here.
pub(crate) fn after_edit<R: Runtime>(app: &AppHandle<R>, doc: &DocumentService) {
    let info: DocumentDto = doc.info().into();
    emit_document_changed(app, &info);
    emit_history_state(app, doc);
}

/// `pub(crate)`: see [`after_edit`].
pub(crate) async fn run_blocking<T: Send + 'static>(
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

// --- S2-01: selection, cut/copy/paste/delete/trim/silence, undo/redo -----------------------

/// Cuts `[start_samples, end_samples)` (SPEC-008 §2.1). `error.no_selection`/`error.invalid_range`
/// for a bad range, `error.not_while_recording` while a take is open.
#[tauri::command]
pub async fn edit_cut<R: Runtime>(
    app: AppHandle<R>,
    doc: State<'_, DocumentService>,
    start_samples: u64,
    end_samples: u64,
) -> Result<EditResultDto, IpcError> {
    let service = (*doc).clone();
    let result: EditResultDto = run_blocking(move || service.edit_cut(start_samples, end_samples))
        .await?
        .into();
    after_edit(&app, &doc);
    emit_clipboard_changed(&app, &doc);
    Ok(result)
}

/// Copies `[start_samples, end_samples)` into the clipboard. Not an edit (no undo entry, no
/// transport stop).
#[tauri::command]
pub async fn edit_copy<R: Runtime>(
    app: AppHandle<R>,
    doc: State<'_, DocumentService>,
    start_samples: u64,
    end_samples: u64,
) -> Result<EditResultDto, IpcError> {
    let service = (*doc).clone();
    let result: EditResultDto = run_blocking(move || service.edit_copy(start_samples, end_samples))
        .await?
        .into();
    emit_clipboard_changed(&app, &doc);
    Ok(result)
}

/// Pastes the clipboard at `target` (SPEC-008 §2.1). `error.clipboard_empty` when nothing was
/// cut/copied yet.
#[tauri::command]
pub async fn edit_paste<R: Runtime>(
    app: AppHandle<R>,
    doc: State<'_, DocumentService>,
    target: EditTargetDto,
) -> Result<EditResultDto, IpcError> {
    let service = (*doc).clone();
    let target: PasteTarget = target.into();
    let result: EditResultDto = run_blocking(move || service.edit_paste(target))
        .await?
        .into();
    after_edit(&app, &doc);
    Ok(result)
}

/// Deletes `[start_samples, end_samples)`, closing the gap.
#[tauri::command]
pub async fn edit_delete<R: Runtime>(
    app: AppHandle<R>,
    doc: State<'_, DocumentService>,
    start_samples: u64,
    end_samples: u64,
) -> Result<EditResultDto, IpcError> {
    let service = (*doc).clone();
    let result: EditResultDto =
        run_blocking(move || service.edit_delete(start_samples, end_samples))
            .await?
            .into();
    after_edit(&app, &doc);
    Ok(result)
}

/// Trims the document to `[start_samples, end_samples)` (Audition: Crop).
#[tauri::command]
pub async fn edit_trim<R: Runtime>(
    app: AppHandle<R>,
    doc: State<'_, DocumentService>,
    start_samples: u64,
    end_samples: u64,
) -> Result<EditResultDto, IpcError> {
    let service = (*doc).clone();
    let result: EditResultDto = run_blocking(move || service.edit_trim(start_samples, end_samples))
        .await?
        .into();
    after_edit(&app, &doc);
    Ok(result)
}

/// Silences `[start_samples, end_samples)` with exact `+0.0` samples.
#[tauri::command]
pub async fn edit_silence<R: Runtime>(
    app: AppHandle<R>,
    doc: State<'_, DocumentService>,
    start_samples: u64,
    end_samples: u64,
) -> Result<EditResultDto, IpcError> {
    let service = (*doc).clone();
    let result: EditResultDto =
        run_blocking(move || service.edit_silence(start_samples, end_samples))
            .await?
            .into();
    after_edit(&app, &doc);
    Ok(result)
}

// S2-02/H-09: peak normalize is now a job (`edit_normalize_peak_start`/`_cancel`,
// `ipc::normalize_commands`) — see that module and `crate::normalize::NormalizeService`.

/// Undoes the top history entry.
#[tauri::command]
pub async fn history_undo<R: Runtime>(
    app: AppHandle<R>,
    doc: State<'_, DocumentService>,
) -> Result<EditResultDto, IpcError> {
    let service = (*doc).clone();
    let result: EditResultDto = run_blocking(move || service.history_undo()).await?.into();
    after_edit(&app, &doc);
    Ok(result)
}

/// Redoes the top history entry.
#[tauri::command]
pub async fn history_redo<R: Runtime>(
    app: AppHandle<R>,
    doc: State<'_, DocumentService>,
) -> Result<EditResultDto, IpcError> {
    let service = (*doc).clone();
    let result: EditResultDto = run_blocking(move || service.history_redo()).await?.into();
    after_edit(&app, &doc);
    Ok(result)
}

// --- S2-03: markers ---------------------------------------------------------------------------

/// The current marker list (SPEC-009 §2.1), in canonical order. Empty (not an error) when no
/// document is open. The UI refetches on every `document_changed` (marker commands emit it too,
/// below), rather than a dedicated `markers_changed` event (S2-03 essential subset).
#[tauri::command]
pub async fn markers_get(doc: State<'_, DocumentService>) -> Result<Vec<MarkerDto>, IpcError> {
    let service = (*doc).clone();
    let markers = run_blocking(move || Ok(service.markers_get())).await?;
    Ok(markers.into_iter().map(Into::into).collect())
}

/// Adds a point (`len_samples == 0`) or region marker (SPEC-009 §2.2). One undo entry
/// `history.marker_add`; never stops playback.
#[tauri::command]
pub async fn marker_add<R: Runtime>(
    app: AppHandle<R>,
    doc: State<'_, DocumentService>,
    pos_samples: u64,
    len_samples: u64,
) -> Result<MarkerDto, IpcError> {
    let service = (*doc).clone();
    let marker: MarkerDto = run_blocking(move || service.marker_add(pos_samples, len_samples))
        .await?
        .into();
    after_edit(&app, &doc);
    Ok(marker)
}

/// Renames marker `id` (SPEC-009 §2.4, normalized server-side). `error.marker_name_empty` when
/// the normalized name is empty.
#[tauri::command]
pub async fn marker_rename<R: Runtime>(
    app: AppHandle<R>,
    doc: State<'_, DocumentService>,
    id: u64,
    name: String,
) -> Result<(), IpcError> {
    let service = (*doc).clone();
    run_blocking(move || service.marker_rename(id, &name)).await?;
    after_edit(&app, &doc);
    Ok(())
}

/// Moves or resizes marker `id` to `[pos_samples, pos_samples + len_samples)` (SPEC-009 §2.5's
/// panel-typed Start/End/Duration edits — dragging is deferred, ticket "Out" list).
#[tauri::command]
pub async fn marker_set_range<R: Runtime>(
    app: AppHandle<R>,
    doc: State<'_, DocumentService>,
    id: u64,
    pos_samples: u64,
    len_samples: u64,
    kind: MarkerRangeKindDto,
) -> Result<(), IpcError> {
    let service = (*doc).clone();
    run_blocking(move || service.marker_set_range(id, pos_samples, len_samples, kind.into()))
        .await?;
    after_edit(&app, &doc);
    Ok(())
}

/// Deletes the markers in `ids` as one undo entry `history.marker_delete` (SPEC-009 §2.6). Delete
/// All/Filtered are deferred (ticket "Out" list).
#[tauri::command]
pub async fn marker_delete<R: Runtime>(
    app: AppHandle<R>,
    doc: State<'_, DocumentService>,
    ids: Vec<u64>,
) -> Result<(), IpcError> {
    let service = (*doc).clone();
    run_blocking(move || service.marker_delete(&ids)).await?;
    after_edit(&app, &doc);
    Ok(())
}

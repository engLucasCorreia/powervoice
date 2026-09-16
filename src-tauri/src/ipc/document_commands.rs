//! Document commands (S1-03): open/save WAV, peaks. Thin: each forwards to [`DocumentService`]
//! and maps the result to a DTO or, for `peaks_get`, raw `VXPK` bytes over `ipc::Response`
//! (ADR-003 §2; CLAUDE.md — bulk IPC data is binary, never JSON float arrays).

use tauri::ipc::Response;
use tauri::{AppHandle, Emitter, Runtime, State};
use vox_project::{ImportProbe, PEAKS_RAW_SPP, VxpkHeader, encode_vxpk};

use crate::document::{DocumentService, PasteJobSource, PasteTarget, document_error};
use crate::ipc::document_dto::{
    ClipboardChangedDto, DocumentDto, DocumentProbeDto, DownmixChoiceDto, EditResultDto,
    EditTargetDto, HistoryStateDto, MarkerDto, MarkerRangeKindDto, PeaksRequestDto,
};
use crate::ipc::error::IpcError;
use crate::ipc::events::EventName;
use crate::ipc::{
    ImportStartedDto, IpcErrorCode, JobKind, JobProgressDto, JobState, Notice, NoticeLevel,
    emit_import_started, emit_job_progress, emit_notice,
};
use crate::settings::{BitDepth, MultichannelPolicy, SaveDitherPref, SettingsStore};

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
///
/// `pub(crate)`, but **not** used by the normalize jobs (S2-02/S4-01): those run on their own
/// thread (`normalize.rs`, driven by `ipc::normalize_commands::edit_normalize_peak_start`/
/// `edit_normalize_lufs_start`) and commit off the async command's own thread, so they can't call
/// this directly — they emit the same `document_changed`/`history_state` shape themselves once a
/// job commits (H-51: this comment used to claim otherwise, naming a command,
/// `loudness_commands::edit_normalize_lufs`, that doesn't exist).
pub(crate) fn after_edit<R: Runtime>(app: &AppHandle<R>, doc: &DocumentService) {
    let info: DocumentDto = doc.info().into();
    emit_document_changed(app, &info);
    emit_history_state(app, doc);
    // T-301 (SPEC-004 §2.5): the disk check runs after every committed edit.
    crate::housekeeping::kick(app);
}

/// `pub(crate)`: see [`after_edit`].
pub(crate) async fn run_blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, IpcError> + Send + 'static,
) -> Result<T, IpcError> {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| IpcError::internal(e.to_string()))?
}

/// T-209 (SPEC-005 §2.4): `probe`'s multichannel input needs the open dialog unless `policy`
/// resolves it on its own — mono and bit-identical-channel input are never ambiguous regardless
/// of `policy` (an identical-channels file opens with the average, itself bit-identical to every
/// channel, SPEC-005 §2.4 "Identical channels").
fn resolve_policy_downmix(
    probe: &ImportProbe,
    policy: MultichannelPolicy,
) -> Option<vox_io::DownmixChoice> {
    if probe.channels.len() <= 1 || probe.identical_channels {
        return Some(vox_io::DownmixChoice::Average);
    }
    match policy {
        MultichannelPolicy::Ask => None,
        MultichannelPolicy::AlwaysMix => Some(vox_io::DownmixChoice::Average),
        MultichannelPolicy::AlwaysFirstChannel => Some(vox_io::DownmixChoice::Channel(0)),
    }
}

/// T-209 (SPEC-005 §2.4): "Open stereo file" — the probe data the dialog needs, carried as a JSON
/// string param (like `error.*`'s existing free-form `message` param) rather than growing
/// `IpcError.params` into a typed union; the frontend `JSON.parse`s it. The UI re-issues
/// `document_open` with `channel_choice` set once the user picks.
fn needs_channel_choice_error(probe: &ImportProbe) -> IpcError {
    let dto = DocumentProbeDto::from(probe.clone());
    let json = serde_json::to_string(&dto).unwrap_or_default();
    IpcError::new(IpcErrorCode::NeedsConfirmation, "dialog.channel_choice")
        .with_param("probe", json)
}

/// Opens `path` as the document (SPEC-005 §2.2-2.4: any container/codec `vox_io::decode`
/// supports — WAV incl. the tolerated variants, FLAC, MP3, M4A AAC-LC, Ogg Vorbis; every other
/// container/codec is `error.open.unsupported_format`/`error.open.unsupported_codec`). Replaces
/// whatever was open — the frontend runs the unsaved-changes prompt (simple version, SPEC-004
/// §2.8) first.
///
/// T-209 (SPEC-005 §2.3/§2.4): probes `path` first. Multichannel input resolves through
/// `channel_choice` (if the UI already asked) or the remembered `multichannel_policy` setting;
/// otherwise this returns `dialog.channel_choice` (see [`needs_channel_choice_error`]) instead of
/// starting the import, and the UI re-issues with the user's choice. Once a downmix is known, the
/// import runs as a job: `job_progress` events (`kind: "import"`) report its progress at
/// SPEC-005 §2.3's rate, and `document_open_cancel(job_id)` cancels it — cancelling, like any
/// other failure, leaves whatever was open before completely untouched (`DocumentService::open`'s
/// existing contract: the new session is only swapped in on success).
#[tauri::command]
pub async fn document_open<R: Runtime>(
    app: AppHandle<R>,
    doc: State<'_, DocumentService>,
    settings: State<'_, SettingsStore>,
    path: String,
    confirm_already_open: bool,
    channel_choice: Option<DownmixChoiceDto>,
) -> Result<DocumentDto, IpcError> {
    let doc = (*doc).clone();
    let doc_for_notice = doc.clone();
    let path_buf = std::path::PathBuf::from(&path);

    // H-20 (SPEC-005 §2.3): probed unconditionally now (not only when a channel choice is still
    // needed) — its `sample_rate_hz`/`len_samples` are also the document shell's, shown to the UI
    // right away via `import_started`, before the (potentially slow) decode loop even starts.
    let probe_path = path_buf.clone();
    let doc_for_probe = doc.clone();
    let probe = run_blocking(move || doc_for_probe.probe(&probe_path)).await?;

    let downmix = match channel_choice {
        Some(choice) => choice.into(),
        None => {
            let policy = settings.get().multichannel_policy;
            match resolve_policy_downmix(&probe, policy) {
                Some(downmix) => downmix,
                None => return Err(needs_channel_choice_error(&probe)),
            }
        }
    };

    let (job_id, cancel) = doc.start_import_job();
    let shell_name = path_buf
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    if let Err(error) = emit_import_started(
        &app,
        ImportStartedDto {
            job_id,
            name: shell_name,
            sample_rate_hz: probe.sample_rate_hz,
            len_samples: probe.len_samples,
        },
    ) {
        tracing::warn!(%error, "emitting import_started failed");
    }
    if let Err(error) = emit_job_progress(
        &app,
        JobProgressDto {
            job_id,
            kind: JobKind::Import,
            state: JobState::Running,
            fraction: 0.0,
        },
    ) {
        tracing::warn!(%error, "emitting the import job's first job_progress failed");
    }
    let doc_for_job = doc.clone();
    let app_for_job = app.clone();
    let path_for_job = path_buf.clone();
    let result = run_blocking(move || {
        let mut last_fraction = 0.0f32;
        doc_for_job.open_with_downmix(
            &path_for_job,
            confirm_already_open,
            downmix,
            &cancel,
            &mut |fraction| {
                // SPEC-005 §2.3: `peaks_progress`/`job_progress` at 4-10 Hz — `import_file`
                // already throttles its own callback to that rate, so every call here is worth
                // forwarding; the small dead-band just skips emitting an event for a fraction
                // that rounds to the one already sent.
                if fraction - last_fraction >= 0.001 || fraction >= 1.0 {
                    last_fraction = fraction;
                    if let Err(error) = emit_job_progress(
                        &app_for_job,
                        JobProgressDto {
                            job_id,
                            kind: JobKind::Import,
                            state: JobState::Running,
                            fraction,
                        },
                    ) {
                        tracing::warn!(%error, "emitting import job_progress failed");
                    }
                }
            },
        )
    })
    .await;
    doc.finish_import_job(job_id);

    match result {
        Ok(info) => {
            let info: DocumentDto = info.into();
            if let Err(error) = emit_job_progress(
                &app,
                JobProgressDto {
                    job_id,
                    kind: JobKind::Import,
                    state: JobState::Done,
                    fraction: 1.0,
                },
            ) {
                tracing::warn!(%error, "emitting the import job's Done job_progress failed");
            }
            emit_document_changed(&app, &info);
            emit_sidecar_notice(&app, &doc_for_notice);
            // SPEC-018 §2.12: only a *successful* open (import completed) reaches here.
            touch_recent_file(&app, &settings, &path_buf);
            Ok(info)
        }
        Err(err) => {
            let state = if err.code == IpcErrorCode::Cancelled {
                JobState::Cancelled
            } else {
                JobState::Failed
            };
            if let Err(error) = emit_job_progress(
                &app,
                JobProgressDto {
                    job_id,
                    kind: JobKind::Import,
                    state,
                    fraction: 0.0,
                },
            ) {
                tracing::warn!(%error, "emitting the import job's terminal job_progress failed");
            }
            Err(err)
        }
    }
}

/// T-209: cancels a running import job (SPEC-005 §2.3 "Cancel"). Best-effort, like every other
/// job's cancel command (H-09/S4-04) — a no-op for an unknown or already-finished job id.
#[tauri::command]
pub async fn document_open_cancel(
    doc: State<'_, DocumentService>,
    job_id: u32,
) -> Result<(), IpcError> {
    doc.cancel_import_job(job_id);
    Ok(())
}

/// T-306 (SPEC-018 §2.12): moves `path` to the top of Settings' `recent_files` and emits
/// `recent_files_changed`. Called only after a successful open/Save As (never a cancelled or
/// failed one, and never for a session/temp path).
fn touch_recent_file<R: Runtime>(
    app: &AppHandle<R>,
    settings: &SettingsStore,
    path: &std::path::Path,
) {
    let mut current = settings.get();
    crate::settings::touch_recent_file(&mut current.recent_files, path);
    match settings.set(current) {
        Ok(saved) => {
            let dtos: Vec<crate::ipc::recent_files_dto::RecentFileDto> =
                saved.recent_files.iter().map(Into::into).collect();
            if let Err(error) = app.emit(EventName::recent_files_changed.as_str(), dtos) {
                tracing::warn!(%error, "emitting recent_files_changed failed");
            }
        }
        Err(error) => tracing::warn!(%error, "recording a recent file failed"),
    }
}

/// T-306/H-20: turns the last open/save's pending sidecar/format notices, if any, into `notice`
/// events (SPEC-018 §2.5/§2.9's notice keys; H-20's `format_mapped`/`metadata_dropped`) — a single
/// save can now surface more than one (e.g. metadata dropped *and* markers not in FLAC).
fn emit_sidecar_notice<R: Runtime>(app: &AppHandle<R>, doc: &DocumentService) {
    use crate::ipc::events::{Notice, NoticeLevel, emit_notice};
    for info in doc.take_sidecar_notices() {
        let level = if info.key.contains("sidecar_failed") {
            NoticeLevel::Error
        } else {
            NoticeLevel::Warning
        };
        let mut notice = Notice::toast(level, info.key);
        for (name, value) in info.params {
            notice = notice.with_param(name, value);
        }
        if let Err(error) = emit_notice(app, notice) {
            tracing::warn!(%error, "emitting a sidecar notice failed");
        }
    }
}

/// Probes `path` without importing it (SPEC-005 §2.3 step 1, §2.4): container/codec/rate/
/// channels, and — for multichannel input — each channel's peak over the first 30 s plus the
/// identical-channels/silent-channel-hint data a future channel-choice dialog (T-209) needs.
/// Errors the same way [`document_open`] would (`error.open.*`), so the frontend can show them
/// before ever starting an import.
#[tauri::command]
pub async fn document_probe(path: String) -> Result<DocumentProbeDto, IpcError> {
    run_blocking(move || {
        vox_project::probe_for_import(std::path::Path::new(&path))
            .map(DocumentProbeDto::from)
            .map_err(crate::document::document_error)
    })
    .await
}

/// Saves the current revision back to its bound path and format (SPEC-005 §2.7). No rack
/// rendering (D-019): this writes exactly the document's audio.
///
/// T-209: `confirm_clip` bypasses SPEC-005 §2.8's clip prompt (the UI re-issues with `true` after
/// "Clip and save"; `dialog.overs`'s `count`/`peak_dbfs` params come from the first refusal).
/// H-20: `confirm_multichannel` bypasses SPEC-005 §2.4's "saving replaces the stereo source with
/// mono" warning (`dialog.multichannel_source`, shown at most once per document).
#[tauri::command]
pub async fn document_save<R: Runtime>(
    app: AppHandle<R>,
    doc: State<'_, DocumentService>,
    overwrite: bool,
    confirm_clip: bool,
    confirm_multichannel: bool,
) -> Result<DocumentDto, IpcError> {
    let doc = (*doc).clone();
    let doc_for_notice = doc.clone();
    let info: DocumentDto =
        run_blocking(move || doc.save(overwrite, confirm_clip, confirm_multichannel))
            .await?
            .into();
    emit_document_changed(&app, &info);
    emit_sidecar_notice(&app, &doc_for_notice);
    Ok(info)
}

/// Saves the current revision to `path` in `container` at `bits`/`dither` (SPEC-005 §2.7), then
/// binds the document to it. T-209: `container`/`bits` reach the saver from the Save As dialog's
/// format row (WAV 16/24/32-bit float or FLAC 16/24); H-20: `dither` is the dialog's Dither row
/// (TPDF/None, shown for 16/24-bit only); `confirm_clip`/`confirm_multichannel` — see
/// [`document_save`]'s doc comment.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn document_save_as<R: Runtime>(
    app: AppHandle<R>,
    doc: State<'_, DocumentService>,
    settings: State<'_, SettingsStore>,
    path: String,
    container: crate::ipc::document_dto::SaveContainerDto,
    bits: BitDepth,
    dither: SaveDitherPref,
    confirm_clip: bool,
    confirm_multichannel: bool,
) -> Result<DocumentDto, IpcError> {
    let doc = (*doc).clone();
    let doc_for_notice = doc.clone();
    let path_buf = std::path::PathBuf::from(&path);
    let info: DocumentDto = run_blocking(move || {
        doc.save_as(
            std::path::Path::new(&path),
            container.into(),
            bits,
            dither,
            confirm_clip,
            confirm_multichannel,
        )
    })
    .await?
    .into();
    emit_document_changed(&app, &info);
    emit_sidecar_notice(&app, &doc_for_notice);
    // SPEC-018 §2.12: Save As always touches the list (a new recording's first save included).
    touch_recent_file(&app, &settings, &path_buf);
    Ok(info)
}

/// T-306 (SPEC-018 §2.6.5): records the spectral pane's current settings for the next save.
/// Fire-and-forget (no result, no `document_changed`): a view-only change never marks the
/// document modified (§2.4), so there is nothing for the title bar or `sidecar_dirty` to react
/// to. The caller (the spectral pane store) debounces its own calls.
#[tauri::command]
pub async fn sidecar_view_set_spectral(
    doc: State<'_, DocumentService>,
    spectral: crate::ipc::document_dto::SpectralViewDto,
) -> Result<(), IpcError> {
    let doc = (*doc).clone();
    run_blocking(move || {
        doc.set_spectral_view(spectral.into());
        Ok(())
    })
    .await
}

/// H-12 (SPEC-018 §2.6.5): records the shared waveform/spectral viewport plus the selection and
/// edit cursor for the next save. Fire-and-forget, same contract as `sidecar_view_set_spectral`
/// (no result, no `document_changed`, never marks the document modified) — the caller
/// (`EditorView`) debounces its own calls.
#[tauri::command]
pub async fn sidecar_view_set_waveform(
    doc: State<'_, DocumentService>,
    waveform: crate::ipc::document_dto::WaveformViewDto,
) -> Result<(), IpcError> {
    let doc = (*doc).clone();
    run_blocking(move || {
        doc.set_waveform_view(waveform.into());
        Ok(())
    })
    .await
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
///
/// H-56 (SPEC-008 §2.6): when the clipboard was materialized from a different, now-closed
/// document and isn't cached for this one yet, this instead runs the cross-document import as a
/// job (§2.6.1): `job_progress` (kind `paste`) reports its progress and `edit_paste_cancel(job_id)`
/// cancels it from another command invocation while this one is still awaiting — cancelling, like
/// any other failure, leaves the document and the clipboard exactly as before.
#[tauri::command]
pub async fn edit_paste<R: Runtime>(
    app: AppHandle<R>,
    doc: State<'_, DocumentService>,
    target: EditTargetDto,
) -> Result<EditResultDto, IpcError> {
    let service = (*doc).clone();
    let target: PasteTarget = target.into();
    let needs_job = {
        let service = service.clone();
        run_blocking(move || Ok(service.paste_needs_job())).await?
    };
    let result: EditResultDto = if needs_job {
        run_cross_document_paste(&app, &service, target).await?
    } else {
        run_blocking(move || service.edit_paste(target))
            .await?
            .into()
    };
    after_edit(&app, &doc);
    emit_clipboard_changed(&app, &doc);
    Ok(result)
}

/// "‹rate› kHz" as `notice.paste_resampled`'s `from`/`to` params (H-56), e.g. `48_000` -> "48
/// kHz", `44_100` -> "44.1 kHz".
fn khz_label(hz: u32) -> String {
    let khz = f64::from(hz) / 1000.0;
    if (khz - khz.round()).abs() < 1e-9 {
        format!("{:.0} kHz", khz.round())
    } else {
        format!("{khz:.1} kHz")
    }
}

/// H-56 (SPEC-008 §2.6.1): the cross-document paste job — begins it (validates the target, marks
/// the document busy), imports the materialized clipboard on a blocking thread with progress
/// events, then commits. Never called for a `Bound` (or already-cached) clipboard; see
/// [`edit_paste`].
async fn run_cross_document_paste<R: Runtime>(
    app: &AppHandle<R>,
    service: &DocumentService,
    target: PasteTarget,
) -> Result<EditResultDto, IpcError> {
    let begin_service = service.clone();
    let source: PasteJobSource =
        run_blocking(move || begin_service.begin_paste_job(target)).await?;
    let (job_id, cancel) = service.start_paste_job();
    if let Err(error) = emit_job_progress(
        app,
        JobProgressDto {
            job_id,
            kind: JobKind::Paste,
            state: JobState::Running,
            fraction: 0.0,
        },
    ) {
        tracing::warn!(%error, "emitting the paste job's first job_progress failed");
    }
    let app_for_job = app.clone();
    let import_source = source.clone();
    let written = run_blocking(move || {
        let mut last_fraction = 0.0f32;
        vox_project::clipboard::import_clip(
            &import_source.clip,
            &import_source.store,
            import_source.target_rate_hz,
            &cancel,
            &mut |fraction| {
                if fraction - last_fraction >= 0.01 || fraction >= 1.0 {
                    last_fraction = fraction;
                    if let Err(error) = emit_job_progress(
                        &app_for_job,
                        JobProgressDto {
                            job_id,
                            kind: JobKind::Paste,
                            state: JobState::Running,
                            fraction,
                        },
                    ) {
                        tracing::warn!(%error, "emitting paste job_progress failed");
                    }
                }
            },
        )
        .map_err(document_error)
    })
    .await;
    service.end_paste_job(job_id);

    let written = match written {
        Ok(written) => written,
        Err(err) => {
            service.abandon_paste_job();
            let state = if err.code == IpcErrorCode::Cancelled {
                JobState::Cancelled
            } else {
                JobState::Failed
            };
            if let Err(error) = emit_job_progress(
                app,
                JobProgressDto {
                    job_id,
                    kind: JobKind::Paste,
                    state,
                    fraction: 0.0,
                },
            ) {
                tracing::warn!(%error, "emitting the paste job's terminal job_progress failed");
            }
            return Err(err);
        }
    };

    let converted = source.clip.sample_rate_hz != source.target_rate_hz;
    let from = source.clip.sample_rate_hz;
    let to = source.target_rate_hz;
    let finish_service = service.clone();
    let finish_source = source.clone();
    let result =
        run_blocking(move || finish_service.finish_paste_job(&finish_source, written)).await;
    match result {
        Ok(result) => {
            if let Err(error) = emit_job_progress(
                app,
                JobProgressDto {
                    job_id,
                    kind: JobKind::Paste,
                    state: JobState::Done,
                    fraction: 1.0,
                },
            ) {
                tracing::warn!(%error, "emitting the paste job's Done job_progress failed");
            }
            if converted {
                let notice = Notice::toast(NoticeLevel::Info, "notice.paste_resampled")
                    .with_param("from", khz_label(from))
                    .with_param("to", khz_label(to));
                if let Err(error) = emit_notice(app, notice) {
                    tracing::warn!(%error, "emitting notice.paste_resampled failed");
                }
            }
            Ok(result.into())
        }
        Err(err) => {
            if let Err(error) = emit_job_progress(
                app,
                JobProgressDto {
                    job_id,
                    kind: JobKind::Paste,
                    state: JobState::Failed,
                    fraction: 0.0,
                },
            ) {
                tracing::warn!(%error, "emitting the paste job's terminal job_progress failed");
            }
            Err(err)
        }
    }
}

/// Cancels a running cross-document paste job (SPEC-008 §2.6.1 "Cancel"). Best-effort, like every
/// other job's cancel command — a no-op for an unknown or already-finished job id.
#[tauri::command]
pub async fn edit_paste_cancel(
    doc: State<'_, DocumentService>,
    job_id: u32,
) -> Result<(), IpcError> {
    doc.cancel_paste_job(job_id);
    Ok(())
}

/// Inserts `len_samples` of silence at `target` (SPEC-008 §2.1/§2.5): at the selection start if
/// one exists, otherwise the cursor. `error.insert_silence_duration` outside the dialog's 1
/// sample .. 3 600 s range.
#[tauri::command]
pub async fn edit_insert_silence<R: Runtime>(
    app: AppHandle<R>,
    doc: State<'_, DocumentService>,
    target: EditTargetDto,
    len_samples: u64,
) -> Result<EditResultDto, IpcError> {
    let service = (*doc).clone();
    let target: PasteTarget = target.into();
    let result: EditResultDto =
        run_blocking(move || service.edit_insert_silence(target, len_samples))
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

#[cfg(test)]
mod tests {
    use vox_project::ImportChannel;

    use super::*;

    fn stereo_probe(identical: bool, suggested: Option<usize>) -> ImportProbe {
        ImportProbe {
            container: "wav".to_string(),
            codec: "pcm".to_string(),
            sample_rate_hz: 48_000,
            channels: vec![
                ImportChannel {
                    label: "Left".to_string(),
                    is_lfe: false,
                },
                ImportChannel {
                    label: "Right".to_string(),
                    is_lfe: false,
                },
            ],
            len_samples: Some(48_000),
            channel_peaks_dbfs: vec![-6.0, -60.0],
            identical_channels: identical,
            suggested_channel: suggested,
            bits_per_sample: Some(24),
            has_foreign_metadata: false,
        }
    }

    fn mono_probe() -> ImportProbe {
        ImportProbe {
            container: "wav".to_string(),
            codec: "pcm".to_string(),
            sample_rate_hz: 48_000,
            channels: vec![ImportChannel {
                label: "Mono".to_string(),
                is_lfe: false,
            }],
            len_samples: Some(48_000),
            channel_peaks_dbfs: Vec::new(),
            identical_channels: false,
            suggested_channel: None,
            bits_per_sample: Some(24),
            has_foreign_metadata: false,
        }
    }

    /// SPEC-005 §2.4: mono input is never ambiguous, whatever the remembered policy.
    #[test]
    fn mono_never_needs_a_dialog() {
        let probe = mono_probe();
        for policy in [
            MultichannelPolicy::Ask,
            MultichannelPolicy::AlwaysMix,
            MultichannelPolicy::AlwaysFirstChannel,
        ] {
            assert_eq!(
                resolve_policy_downmix(&probe, policy),
                Some(vox_io::DownmixChoice::Average)
            );
        }
    }

    /// SPEC-005 §2.4 "Identical channels": no dialog even when the policy is `Ask`.
    #[test]
    fn identical_channels_never_need_a_dialog_even_when_asking() {
        let probe = stereo_probe(true, None);
        assert_eq!(
            resolve_policy_downmix(&probe, MultichannelPolicy::Ask),
            Some(vox_io::DownmixChoice::Average)
        );
    }

    /// SPEC-005 §2.4/AC-10: `Ask` on genuinely ambiguous stereo input needs the dialog.
    #[test]
    fn ambiguous_stereo_with_ask_needs_the_dialog() {
        let probe = stereo_probe(false, Some(0));
        assert_eq!(
            resolve_policy_downmix(&probe, MultichannelPolicy::Ask),
            None
        );
    }

    /// SPEC-005 §2.4/AC-10: "With `multichannel_policy` = always mix, no dialog appears."
    #[test]
    fn always_mix_policy_skips_the_dialog() {
        let probe = stereo_probe(false, None);
        assert_eq!(
            resolve_policy_downmix(&probe, MultichannelPolicy::AlwaysMix),
            Some(vox_io::DownmixChoice::Average)
        );
    }

    /// SPEC-005 §2.4: "always use the first channel" picks index 0 regardless of the silent-
    /// channel hint's own suggestion (that hint only drives the dialog itself).
    #[test]
    fn always_first_channel_policy_picks_index_zero() {
        let probe = stereo_probe(false, Some(1));
        assert_eq!(
            resolve_policy_downmix(&probe, MultichannelPolicy::AlwaysFirstChannel),
            Some(vox_io::DownmixChoice::Channel(0))
        );
    }

    /// `needs_channel_choice_error` carries the probe as a JSON string param the frontend can
    /// parse back into the dialog's data (channels, peaks, the silent-channel hint).
    #[test]
    fn needs_channel_choice_error_carries_the_probe_as_json() {
        let probe = stereo_probe(false, Some(0));
        let err = needs_channel_choice_error(&probe);
        assert_eq!(err.code, IpcErrorCode::NeedsConfirmation);
        assert_eq!(err.key, "dialog.channel_choice");
        let json = err.params.get("probe").expect("a probe param");
        let parsed: DocumentProbeDto = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.channels.len(), 2);
        assert_eq!(parsed.suggested_channel, Some(0));
        assert!((parsed.channel_peaks_dbfs[0] - (-6.0)).abs() < 1e-9);
    }
}

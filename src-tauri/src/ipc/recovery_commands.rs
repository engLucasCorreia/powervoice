//! Crash recovery & session lifecycle commands (T-301, SPEC-004 §2.7, §2.8). Thin: each forwards
//! to [`DocumentService`] (which forwards to `vox_project::Session::recover` / `gc`).

use tauri::{AppHandle, Emitter, Runtime, State};

use crate::document::DocumentService;
use crate::ipc::document_dto::{DocumentDto, HistoryStateDto};
use crate::ipc::error::IpcError;
use crate::ipc::events::{EventName, Notice, NoticeLevel, emit_notice};
use crate::ipc::recovery_dto::{
    RecoverResultDto, RecoverableSessionDto, RecoveredTakeActionDto, StorageInfoDto,
};

use super::document_commands::run_blocking;

fn list(doc: &DocumentService) -> Vec<RecoverableSessionDto> {
    doc.recovery_list()
        .iter()
        .map(RecoverableSessionDto::from)
        .collect()
}

/// The recoverable sessions (start-up dialog, Settings → Recovery & storage). Also finishes
/// interrupted deletions and removes cleanly closed sessions (SPEC-004 §2.8).
#[tauri::command]
pub async fn recovery_list(
    doc: State<'_, DocumentService>,
) -> Result<Vec<RecoverableSessionDto>, IpcError> {
    let doc = (*doc).clone();
    run_blocking(move || Ok(list(&doc))).await
}

/// Recover (SPEC-004 §2.7): opens session `id` as the document, handling its interrupted take
/// per `take_action`. Emits `document_changed` and `history_state`, plus a "The last N changes
/// could not be recovered" notice when records were lost.
#[tauri::command]
pub async fn recovery_recover<R: Runtime>(
    app: AppHandle<R>,
    doc: State<'_, DocumentService>,
    id: String,
    take_action: RecoveredTakeActionDto,
) -> Result<RecoverResultDto, IpcError> {
    let service = (*doc).clone();
    let result: RecoverResultDto = run_blocking(move || service.recover(&id, take_action.into()))
        .await?
        .into();
    if let Err(error) = app.emit(
        EventName::document_changed.as_str(),
        result.document.clone(),
    ) {
        tracing::warn!(%error, "emitting document_changed failed");
    }
    let history: HistoryStateDto = doc.history_state().into();
    if let Err(error) = app.emit(EventName::history_state.as_str(), history) {
        tracing::warn!(%error, "emitting history_state failed");
    }
    if result.lost_changes > 0 {
        let notice = Notice::toast(NoticeLevel::Warning, "notice.recovery.lost_changes")
            .with_param("count", result.lost_changes.to_string());
        if let Err(error) = emit_notice(&app, notice) {
            tracing::warn!(%error, "emitting the recovery notice failed");
        }
    }
    Ok(result)
}

/// Discard (after the UI's confirmation): permanently deletes session `id`, never one in use.
/// Returns the remaining list.
#[tauri::command]
pub async fn recovery_discard(
    doc: State<'_, DocumentService>,
    id: String,
) -> Result<Vec<RecoverableSessionDto>, IpcError> {
    let doc = (*doc).clone();
    run_blocking(move || {
        doc.recovery_discard(&id)?;
        Ok(list(&doc))
    })
    .await
}

/// Settings → Recovery & storage's figures.
#[tauri::command]
pub async fn storage_info(doc: State<'_, DocumentService>) -> Result<StorageInfoDto, IpcError> {
    let doc = (*doc).clone();
    run_blocking(move || {
        let usage = doc.storage_usage();
        let sessions = list(&doc);
        Ok(StorageInfoDto {
            session_bytes: usage.as_ref().map(|u| u.session_bytes),
            history_bytes: usage.as_ref().map(|u| u.history_bytes),
            recovery_bytes: sessions.iter().map(|s| s.size_bytes).sum(),
            sessions,
        })
    })
    .await
}

/// Closes the document for good after the quit prompt's Save or Don't Save (SPEC-004 §2.8):
/// the session directory is deleted, so a discarded document is never offered for recovery.
#[tauri::command]
pub async fn document_close<R: Runtime>(
    app: AppHandle<R>,
    doc: State<'_, DocumentService>,
) -> Result<(), IpcError> {
    let service = (*doc).clone();
    run_blocking(move || service.close()).await?;
    let info: DocumentDto = doc.info().into();
    if let Err(error) = app.emit(EventName::document_changed.as_str(), info) {
        tracing::warn!(%error, "emitting document_changed failed");
    }
    Ok(())
}

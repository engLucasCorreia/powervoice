//! T-204 spectrogram tile commands (ADR-003 §2). Thin: `spectro_attach` binds a spectral view's
//! channel to the engine's [`SpectroService`]; `spectro_request` forwards a request together with
//! the current document snapshot. Tiles stream back as binary `VXST` frames on the channel, one
//! message per tile (CLAUDE.md: bulk IPC data is binary, never JSON float arrays).

use std::sync::Arc;

use tauri::State;
use tauri::ipc::{Channel, InvokeResponseBody};
use vox_engine::spectro::{SpectroError, SpectroService};

use crate::document::DocumentService;
use crate::ipc::document_commands::run_blocking;
use crate::ipc::error::{IpcError, IpcErrorCode};
use crate::ipc::spectro_dto::SpectroRequestDto;

fn spectro_error(err: SpectroError) -> IpcError {
    IpcError::new(IpcErrorCode::InvalidArgument, "error.invalid_argument")
        .with_param("message", err.to_string())
}

/// Attaches spectral view `view_id`: its tiles go to `channel` from now on (replacing a previous
/// channel of the same view and cancelling that one's pending tiles).
#[tauri::command]
pub async fn spectro_attach(
    spectro: State<'_, Arc<SpectroService>>,
    view_id: u32,
    channel: Channel,
) -> Result<(), IpcError> {
    spectro.attach(
        view_id,
        Box::new(move |bytes| {
            let _ = channel.send(InvokeResponseBody::Raw(bytes));
        }),
    );
    Ok(())
}

/// Detaches spectral view `view_id` (its pending tiles are cancelled).
#[tauri::command]
pub async fn spectro_detach(
    spectro: State<'_, Arc<SpectroService>>,
    view_id: u32,
) -> Result<(), IpcError> {
    spectro.detach(view_id);
    Ok(())
}

/// Requests tiles of the current document for view `view_id` (SPEC-007 §4.6). A newer request
/// cancels the view's older one. `error.document.none` without a document,
/// `error.invalid_argument` for an unattached view or invalid FFT size/hop/window.
#[tauri::command]
pub async fn spectro_request(
    spectro: State<'_, Arc<SpectroService>>,
    doc: State<'_, DocumentService>,
    view_id: u32,
    request: SpectroRequestDto,
) -> Result<(), IpcError> {
    let spectro = Arc::clone(&spectro);
    let doc = (*doc).clone();
    run_blocking(move || {
        let source = doc.spectro_source()?;
        spectro
            .request(view_id, source, request.into())
            .map_err(spectro_error)
    })
    .await
}

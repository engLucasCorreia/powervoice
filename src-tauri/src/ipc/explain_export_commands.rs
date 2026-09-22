//! "Explain My Voice" export (H-115, security amendment): writes the bytes the frontend already
//! rendered — a PNG image or a self-contained HTML report — to a path chosen through the native
//! save dialog (`tauri-plugin-dialog`, already used from the frontend elsewhere; here opened from
//! the backend instead).
//!
//! **Why the dialog moved to the backend.** The first version of this command took `path: String`
//! straight from the frontend. In normal use that path came from a native save dialog, but the
//! command itself enforced nothing: any script running in the webview — not necessarily this
//! feature's own code, a future injection bug anywhere in the app — could call
//! `explain_export_write` with an arbitrary path and overwrite any file the user can write. An
//! export feature must not add a general "write any bytes to any path" capability to the webview.
//! Now the backend owns the destination end to end: [`explain_export_pick_path`] opens the dialog
//! itself (the frontend only supplies UI hints — a suggested file name and a filter — never a
//! path) and, whatever the user actually typed or picked, checks the *chosen* path's extension
//! against [`ALLOWED_EXTENSIONS`], a hardcoded allow-list independent of the filter the frontend
//! asked the dialog to display (a native dialog still lets a user override the filter and type any
//! name). Only then does it mint a single-use token bound to that exact [`PathBuf`], stored in
//! [`ExplainExportTokens`] — nothing the frontend can turn back into a path.
//!
//! **Why the bytes are a raw body, not a `path`/`bytes` JSON command.** A real, maximised,
//! large-display export (`ExplainGraph`'s own live canvas + a realistic ~9-card/10-finding
//! report — see this module's `bench_measurement` notes below) renders a ~1.0 MB PNG. As a JSON
//! number array that becomes ~3.6 MB of decimal text (`Array.from(bytes)` + `JSON.stringify`
//! measured at ~55 ms of pure JS marshalling on top of the ~80 ms render, in a real headless
//! Chromium against the actual `renderExplainExportPng`/real graph canvas — see the ticket
//! amendment) — noticeable, and it gets worse exactly where this ticket made it more likely: a
//! bigger dialog on a bigger display renders a bigger PNG. CLAUDE.md's binary-IPC rule applies:
//! [`explain_export_write_bytes`] takes the bytes as the *entire* request body
//! (`tauri::ipc::Request`, sent from the frontend as a `Uint8Array` — `@tauri-apps/api/core`'s
//! `invoke` sends any `ArrayBuffer`/`Uint8Array`/`number[]` payload as `application/octet-stream`,
//! never JSON, by design), with the token carried in a header instead of a command argument — a
//! JSON command can't also carry a raw body, so metadata that must travel alongside one goes in a
//! header, exactly the way Tauri's own `Channel` fetches its queued data
//! (`tauri::ipc::channel`'s internal `fetch` command reads `Tauri-Channel-Id` off `Request`).
//!
//! Redeeming a token (successfully or not) removes it — it can never be reused for a second
//! write, so a script cannot replay one write call to hit the same destination twice, and cannot
//! address any destination the user has not just picked in a real, visible native dialog.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use tauri::ipc::{InvokeBody, Request};
use tauri::{AppHandle, State};
use tauri_plugin_dialog::DialogExt;

use crate::ipc::error::{IpcError, IpcErrorCode};

/// The only extensions this command will ever write to — checked against the path the save
/// dialog actually returned, not the filter the frontend asked it to show (module doc).
const ALLOWED_EXTENSIONS: &[&str] = &["png", "html"];

/// The header `explain_export_write_bytes` reads its token from (module doc: mirrors
/// `tauri::ipc::channel`'s own `Tauri-Channel-Id`).
const TOKEN_HEADER: &str = "X-Explain-Export-Token";

/// Single-use dialog-picked destinations, keyed by an opaque token minted when the dialog returns
/// a path. Not a secret across a network boundary — the security property comes from *only*
/// existing here for a path a real native dialog just returned, and from being removed the moment
/// it's redeemed (`take`) — so a per-process counter plus a timestamp is enough entropy; this is
/// not trying to be a CSPRNG (no new dependency for one).
#[derive(Default)]
pub struct ExplainExportTokens(Mutex<HashMap<String, PathBuf>>);

/// The app handle, managed once at start-up (`lib.rs`) so `explain_export_pick_path` can reach it
/// through `State` — the ordinary pattern every command in this crate uses — instead of taking
/// `tauri::AppHandle` directly as a command argument, which needs the command function to be
/// generic over the runtime type (`AppHandle<R>` where `R: tauri::Runtime`, not the bare
/// `AppHandle` a plain `#[tauri::command]` fn can't resolve against the macro's own generic `R`)
/// for more machinery than one dialog call is worth.
pub struct ExplainExportAppHandle(pub AppHandle);

impl ExplainExportTokens {
    fn mint(&self, path: PathBuf) -> String {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let token = format!("{nanos:x}-{n:x}");
        self.0.lock().unwrap().insert(token.clone(), path);
        token
    }

    /// Removes and returns the path for `token`, or `None` for an unknown/already-redeemed one —
    /// the single-use guarantee (module doc).
    fn take(&self, token: &str) -> Option<PathBuf> {
        self.0.lock().unwrap().remove(token)
    }
}

/// Whether `path`'s extension (case-insensitive) is one this feature may ever write to — the
/// hardcoded second check the module doc describes, independent of whatever filter a caller asked
/// the dialog to display.
fn allowed_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| ALLOWED_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
}

/// Opens the native save dialog with `suggested_file_name`/`filter_name`/`filter_extensions` (UI
/// hints only). Returns `Ok(None)` if the user cancelled; refuses (without minting a token) a
/// chosen path whose extension isn't in [`ALLOWED_EXTENSIONS`], whatever filter was requested.
#[tauri::command]
pub async fn explain_export_pick_path(
    app: State<'_, ExplainExportAppHandle>,
    tokens: State<'_, ExplainExportTokens>,
    suggested_file_name: String,
    filter_name: String,
    filter_extensions: Vec<String>,
) -> Result<Option<String>, IpcError> {
    let extension_refs: Vec<&str> = filter_extensions.iter().map(String::as_str).collect();
    let picked = app
        .0
        .dialog()
        .file()
        .add_filter(filter_name, &extension_refs)
        .set_file_name(suggested_file_name)
        .blocking_save_file();
    let Some(file_path) = picked else {
        return Ok(None);
    };
    let path = file_path
        .into_path()
        .map_err(|e| IpcError::internal(e.to_string()))?;
    if !allowed_extension(&path) {
        return Err(IpcError::new(
            IpcErrorCode::InvalidArgument,
            "error.explain_export.bad_extension",
        ));
    }
    Ok(Some(tokens.mint(path)))
}

/// Writes the request's raw body to the path `token` was minted for (module doc). The token is
/// redeemed (removed) as soon as it's looked up, whether or not the write that follows succeeds.
#[tauri::command]
pub async fn explain_export_write_bytes(
    request: Request<'_>,
    tokens: State<'_, ExplainExportTokens>,
) -> Result<(), IpcError> {
    let token = request
        .headers()
        .get(TOKEN_HEADER)
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| {
            IpcError::new(
                IpcErrorCode::InvalidArgument,
                "error.explain_export.missing_token",
            )
        })?
        .to_string();
    let bytes: Vec<u8> = match request.body() {
        InvokeBody::Raw(bytes) => bytes.clone(),
        InvokeBody::Json(_) => {
            return Err(IpcError::new(
                IpcErrorCode::InvalidArgument,
                "error.explain_export.not_binary",
            ));
        }
    };
    // `request` (and its borrow of the invoke message) is no longer needed past this point —
    // nothing above is held across the `.await` below.

    let path = tokens.take(&token).ok_or_else(|| {
        IpcError::new(
            IpcErrorCode::InvalidArgument,
            "error.explain_export.invalid_token",
        )
    })?;
    // Second check, independent of the one `explain_export_pick_path` already ran: the
    // token/path pair never left the backend so this can't currently fail, but it costs nothing
    // and keeps the guarantee "this command only ever writes .png/.html" true even if a future
    // change adds another way to reach this point.
    if !allowed_extension(&path) {
        return Err(IpcError::new(
            IpcErrorCode::InvalidArgument,
            "error.explain_export.bad_extension",
        ));
    }

    let result = write_export_bytes(&path, bytes);
    result.map_err(IpcError::from)
}

/// The actual write. Not `spawn_blocking`-wrapped: unlike the old single-command shape, most of
/// this command's own work (the token lookup, the checks) is already synchronous and cheap, and
/// splitting one `std::fs::write` onto another thread bought nothing a unit test could show —
/// kept as a plain function so it's still directly `#[test]`-able without an app/dialog/runtime,
/// the same pattern this crate's other command tests use (e.g. `recent_files_commands.rs`'s
/// `checked_existence`).
fn write_export_bytes(path: &Path, bytes: Vec<u8>) -> std::io::Result<()> {
    std::fs::write(path, bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_the_given_bytes_to_the_given_path() {
        let dir = crate::test_util::tmp_dir("explain-export-write");
        let path = dir.join("analysis.png");
        let bytes = vec![1u8, 2, 3, 4, 5];

        write_export_bytes(&path, bytes.clone()).unwrap();

        assert_eq!(std::fs::read(&path).unwrap(), bytes);
    }

    #[test]
    fn overwrites_an_existing_file() {
        let dir = crate::test_util::tmp_dir("explain-export-overwrite");
        let path = dir.join("report.html");
        std::fs::write(&path, b"old").unwrap();

        write_export_bytes(&path, b"new content".to_vec()).unwrap();

        assert_eq!(std::fs::read(&path).unwrap(), b"new content");
    }

    #[test]
    fn a_directory_that_does_not_exist_is_reported_as_an_io_error() {
        let path = Path::new("/nonexistent-h115-dir/out.png");

        let err = write_export_bytes(path, vec![1, 2, 3]).unwrap_err();

        assert_eq!(err.kind(), std::io::ErrorKind::NotFound);
    }

    #[test]
    fn allowed_extension_accepts_only_png_and_html_case_insensitively() {
        assert!(allowed_extension(Path::new("out.png")));
        assert!(allowed_extension(Path::new("out.PNG")));
        assert!(allowed_extension(Path::new("out.html")));
        assert!(allowed_extension(Path::new("OUT.HTML")));
    }

    #[test]
    fn allowed_extension_refuses_anything_else() {
        assert!(!allowed_extension(Path::new("out.sh")));
        assert!(!allowed_extension(Path::new("out.desktop")));
        assert!(!allowed_extension(Path::new(".bashrc")));
        assert!(!allowed_extension(Path::new("out")));
        assert!(!allowed_extension(Path::new("out.png.sh")));
    }

    #[test]
    fn a_minted_token_resolves_to_its_path_exactly_once() {
        let tokens = ExplainExportTokens::default();
        let token = tokens.mint(PathBuf::from("/home/user/analysis.png"));

        assert_eq!(
            tokens.take(&token),
            Some(PathBuf::from("/home/user/analysis.png"))
        );
        // Single-use: the same token cannot be redeemed twice.
        assert_eq!(tokens.take(&token), None);
    }

    #[test]
    fn an_unknown_token_resolves_to_nothing() {
        let tokens = ExplainExportTokens::default();
        assert_eq!(tokens.take("not-a-real-token"), None);
    }

    #[test]
    fn two_pending_tokens_stay_independent() {
        let tokens = ExplainExportTokens::default();
        let a = tokens.mint(PathBuf::from("/home/user/a.png"));
        let b = tokens.mint(PathBuf::from("/home/user/b.html"));

        assert_ne!(a, b);
        assert_eq!(tokens.take(&a), Some(PathBuf::from("/home/user/a.png")));
        // Taking `a` never affects `b`'s still-pending entry.
        assert_eq!(tokens.take(&b), Some(PathBuf::from("/home/user/b.html")));
    }
}

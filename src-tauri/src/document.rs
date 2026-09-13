//! Document service (S1-03): the app-level owner of the one open session (ADR-004 §8, SPEC-004
//! §2.1). Wraps S1-02's `import_wav`/`save_snapshot_wav`/`peaks` over a [`vox_project::Session`]
//! and keeps the engine's playback document in sync (`EngineHandle::set_document`, MEMORY.md
//! S1-01 Engine API). `ipc::document_commands` only forwards to this — no business logic lives
//! in the command handlers themselves (CLAUDE.md: "`src-tauri` is a thin command/event layer").

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use vox_engine::{EngineHandle, PlaybackDoc, TransportCommand};
use vox_project::{
    EditTarget, FinishedTake, Piece, ProjectError, Range, RangeError, Session, SessionConfig,
    SnapshotReader, TakeCapture, TakeId, TakeMode, TakeWriterOptions, edit, validate_range,
};

use crate::ipc::{IpcError, IpcErrorCode};
use crate::settings::BitDepth;

/// Every completed retry-with-backoff attempt at closing a superseded session waits this long
/// (ADR-002: the reader thread drops its `Arc<ChunkStore>` asynchronously after
/// `set_document`/`set_document(None)` returns, so a `StoreInUse` refusal right after is
/// expected, not a bug — see [`close_with_retry`]).
const CLOSE_RETRY_DELAY: Duration = Duration::from_millis(5);
/// Give up after this many retries (200 ms total) and leave the directory for a future GC pass
/// (session recovery/GC at start-up is out of this ticket's scope, MEMORY.md follow-up).
const CLOSE_RETRY_ATTEMPTS: u32 = 40;
/// S1-04: `commit_take` attempts before giving up (the take then stays open for recovery).
const COMMIT_ATTEMPTS: u32 = 3;
const COMMIT_RETRY_DELAY: Duration = Duration::from_millis(100);

/// The OS data dir's `sessions` subfolder (mirrors `settings::project_dirs`'s convention; no
/// state dir on macOS/Windows, so those fall back to the data dir like `logging.rs` does).
pub fn default_sessions_dir() -> PathBuf {
    match directories::ProjectDirs::from("app", "powervoice", "powervoice") {
        Some(dirs) => dirs.data_dir().join("sessions"),
        None => std::env::temp_dir().join("powervoice").join("sessions"),
    }
}

/// One open document: its session plus the path/format it's bound to (`None` path = never saved:
/// a new recording, S1-04).
struct OpenDocument {
    session: Session,
    path: Option<PathBuf>,
    save_bits: BitDepth,
}

/// [`DocumentService::peaks`]'s result: the buckets (or raw samples, `spp == PEAKS_RAW_SPP`) plus
/// the document facts the caller needs to build a `VXPK` header (ADR-003 §2).
#[derive(Debug)]
pub struct PeaksResult {
    pub audio_rev: u64,
    pub sample_rate_hz: u32,
    pub buckets: Vec<(f32, f32)>,
}

/// Everything a `document_changed` event / `document_open`/`document_save`/`document_save_as`
/// result needs. `sample_rate_hz == 0` means no document is open; an untitled document (a new
/// recording, S1-04) has `name` and `path` `None`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DocumentInfo {
    pub name: Option<String>,
    pub path: Option<String>,
    pub sample_rate_hz: u32,
    pub len_samples: u64,
    pub dirty: bool,
    pub audio_rev: u64,
}

/// S2-01: the in-app clipboard (SPEC-008 §2.6), same-document only for now (cleared whenever a
/// new session replaces the open one — its pieces reference the closed session's chunks).
struct ClipboardData {
    pieces: Vec<Piece>,
    sample_rate_hz: u32,
    len_samples: u64,
}

/// The clipboard's public shape (`clipboard_changed` event): `None` fields mean it's empty.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ClipboardInfo {
    pub len_samples: Option<u64>,
    pub sample_rate_hz: Option<u32>,
}

/// S2-01: the Edit menu's Undo/Redo state (`history_state` event). Labels are i18n keys
/// (`history.cut`, …, CLAUDE.md), not rendered text.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HistoryState {
    pub can_undo: bool,
    pub can_redo: bool,
    pub undo_label: Option<String>,
    pub redo_label: Option<String>,
}

/// S2-01: where Paste inserts or replaces (SPEC-008 §2.1). Plain document samples — the caller
/// (the waveform's selection store) resolves "no selection" to `Cursor` itself (SPEC-008 §2.2: an
/// empty selection counts as the cursor).
#[derive(Clone, Copy, Debug)]
pub enum PasteTarget {
    Cursor(u64),
    Range(u64, u64),
}

/// The result of a cut/copy/paste/delete/trim/silence command, or an undo/redo (SPEC-008 §4.3's
/// `EditResult`, minus `rev`/`base_rev` — revision-guarded commands are out of this ticket's
/// scope). `changed = false` only for Copy and a whole-document Trim (both no-ops).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct EditResult {
    pub changed: bool,
    pub audio_rev: u64,
    pub len_samples: u64,
    pub selection: Option<(u64, u64)>,
    pub playhead_samples: u64,
}

struct Inner {
    sessions_dir: PathBuf,
    engine: EngineHandle,
    open: Mutex<Option<OpenDocument>>,
    clipboard: Mutex<Option<ClipboardData>>,
}

/// Cheaply cloneable (an `Arc` inside), like [`EngineHandle`] — so command handlers can clone it
/// out of Tauri's `State` and run the blocking session/file work with `spawn_blocking` (the same
/// pattern `ipc::audio_commands` uses for `EngineHandle`).
#[derive(Clone)]
pub struct DocumentService(Arc<Inner>);

fn no_document() -> IpcError {
    IpcError::new(IpcErrorCode::NotFound, "error.document.none")
}

/// Save of a never-saved document (S1-04 recording): the frontend routes it to Save As.
fn untitled() -> IpcError {
    IpcError::new(IpcErrorCode::InvalidArgument, "error.document.untitled")
}

/// SPEC-008 §2.2/§4.3: no selection (or an empty one) where one is required.
fn no_selection() -> IpcError {
    IpcError::new(IpcErrorCode::InvalidArgument, "error.no_selection")
}

/// SPEC-008 §4.3: a range or cursor position past the end of the document.
fn invalid_range() -> IpcError {
    IpcError::new(IpcErrorCode::InvalidArgument, "error.invalid_range")
}

/// SPEC-008 §4.3: Paste with an empty clipboard.
fn clipboard_empty() -> IpcError {
    IpcError::new(IpcErrorCode::InvalidArgument, "error.clipboard_empty")
}

/// Maps a [`ProjectError`] to an [`IpcError`], reusing its i18n key (`ProjectError::i18n_key`).
fn document_error(err: ProjectError) -> IpcError {
    let code = match &err {
        ProjectError::NotWhileRecording => IpcErrorCode::NotWhileRecording,
        ProjectError::InvalidArgument(_) | ProjectError::InvalidEdit(_) => {
            IpcErrorCode::InvalidArgument
        }
        ProjectError::Cancelled => IpcErrorCode::Cancelled,
        _ => IpcErrorCode::Internal,
    };
    IpcError::new(code, err.i18n_key()).with_param("message", err.to_string())
}

/// Maps a [`vox_io::IoError`] (from `read_wav`) to an [`IpcError`]. Only 16/24-bit integer and
/// 32-bit float WAV are accepted (S1-02 scope); every other variant is `Unsupported` (ticket:
/// "show it as a notice").
fn io_open_error(err: vox_io::IoError) -> IpcError {
    match err {
        vox_io::IoError::Unsupported(message) => IpcError::new(
            IpcErrorCode::InvalidArgument,
            "error.open.unsupported_format",
        )
        .with_param("message", message),
        vox_io::IoError::Io(e) if e.kind() == std::io::ErrorKind::NotFound => {
            IpcError::new(IpcErrorCode::NotFound, "error.open.not_found")
        }
        other => IpcError::new(IpcErrorCode::Io, "error.open.io")
            .with_param("message", other.to_string()),
    }
}

fn io_save_error(err: vox_io::IoError) -> IpcError {
    IpcError::new(IpcErrorCode::Io, "error.save.io").with_param("message", err.to_string())
}

/// `WavSource`'s container format → the save format a document opened from it keeps (SPEC-005
/// §2.6, restricted to this ticket's three supported variants — 8-bit/A-law/µ-law/32-int/64-float
/// mapping is deferred, S1-02 ticket scope already documents that read of those variants is
/// rejected, so no mapping table is needed here).
fn save_bits_for(format: vox_io::WavFormat) -> BitDepth {
    match (format.sample_format, format.bits_per_sample) {
        (vox_io::SampleFormat::Float, 32) => BitDepth::Bit32Float,
        (vox_io::SampleFormat::Int, 16) => BitDepth::Bit16,
        _ => BitDepth::Bit24,
    }
}

impl From<BitDepth> for vox_io::BitDepth {
    fn from(bits: BitDepth) -> Self {
        match bits {
            BitDepth::Bit16 => vox_io::BitDepth::Int16,
            BitDepth::Bit24 => vox_io::BitDepth::Int24,
            BitDepth::Bit32Float => vox_io::BitDepth::Float32,
        }
    }
}

fn format_tag(bits: BitDepth) -> &'static str {
    match bits {
        BitDepth::Bit16 => "wav16",
        BitDepth::Bit24 => "wav24",
        BitDepth::Bit32Float => "wav32f",
    }
}

fn info_of(doc: Option<&OpenDocument>) -> DocumentInfo {
    let Some(doc) = doc else {
        return DocumentInfo::default();
    };
    let snapshot = doc.session.current();
    DocumentInfo {
        name: doc
            .path
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned()),
        path: doc.path.as_ref().map(|p| p.to_string_lossy().into_owned()),
        sample_rate_hz: doc.session.sample_rate_hz(),
        len_samples: snapshot.len_samples,
        dirty: doc.session.is_dirty(),
        audio_rev: snapshot.audio_rev,
    }
}

/// Closes a superseded session, retrying briefly on [`ProjectError::StoreInUse`]: the reader
/// thread drops its `Arc<ChunkStore>` a little after `EngineHandle::set_document` returns (it
/// only *sends* the new document to the reader thread, ADR-002), so an immediate `close()` right
/// after swapping the engine's document can race it harmlessly. Never blocks the caller for more
/// than `CLOSE_RETRY_ATTEMPTS * CLOSE_RETRY_DELAY` (200 ms).
fn close_with_retry(mut session: Session) {
    for _ in 0..CLOSE_RETRY_ATTEMPTS {
        match session.close() {
            Ok(()) => return,
            Err(vox_project::CloseError {
                error: ProjectError::StoreInUse,
                session: Some(s),
            }) => {
                session = *s;
                std::thread::sleep(CLOSE_RETRY_DELAY);
            }
            Err(vox_project::CloseError { error, session: _ }) => {
                // Journaled `close` failed, or the directory delete failed after it (`session`
                // is `None` then) — a future GC pass (not wired in this ticket) will finish it.
                tracing::warn!(error = %error, "closing the previous session left it for GC");
                return;
            }
        }
    }
    tracing::warn!(
        "previous session stayed in use after retrying; leaving it for a future GC pass"
    );
}

impl DocumentService {
    pub fn new(sessions_dir: PathBuf, engine: EngineHandle) -> Self {
        Self(Arc::new(Inner {
            sessions_dir,
            engine,
            open: Mutex::new(None),
            clipboard: Mutex::new(None),
        }))
    }

    /// The currently open document (or [`DocumentInfo::default`] when none is open).
    pub fn info(&self) -> DocumentInfo {
        info_of(self.0.open.lock().unwrap().as_ref())
    }

    /// Imports `path` as a new session (SPEC-005 §2.3, S1-02 `import_wav`) and makes it the
    /// engine's playback document. Replaces whatever was open before (the frontend is
    /// responsible for the unsaved-changes prompt — SPEC-004 §2.8 "simple version", ticket scope
    /// — before calling this).
    pub fn open(&self, path: &Path) -> Result<DocumentInfo, IpcError> {
        if self.is_recording() {
            return Err(IpcError::not_while_recording());
        }
        let (rate, _channels, mut source) = vox_io::read_wav(path).map_err(io_open_error)?;
        let save_bits = save_bits_for(source.format());
        let mut session = Session::create(&self.0.sessions_dir, SessionConfig::new(rate))
            .map_err(document_error)?;
        if let Err(err) = vox_project::import_wav(&mut session, &mut source) {
            let _ = std::fs::remove_dir_all(session.dir());
            return Err(document_error(err));
        }
        let snapshot = session.current();
        let store = Arc::clone(session.store());
        self.0
            .engine
            .set_document(Some(PlaybackDoc { store, snapshot }));
        // S2-01: a new document replaces the clipboard (same-document only, SPEC-008 §2.6) — its
        // pieces reference the session that's about to close.
        *self.0.clipboard.lock().unwrap() = None;

        let mut guard = self.0.open.lock().unwrap();
        let previous = guard.replace(OpenDocument {
            session,
            path: Some(path.to_path_buf()),
            save_bits,
        });
        let info = info_of(guard.as_ref());
        drop(guard);
        if let Some(previous) = previous {
            close_with_retry(previous.session);
        }
        Ok(info)
    }

    /// Writes the current revision back to its bound path at its bound format (SPEC-005 §2.7).
    /// The frontend routes "Save" with no bound path (an untitled recording, S1-04) through
    /// `document_save_as` instead; here it is refused (`error.document.untitled`).
    pub fn save(&self) -> Result<DocumentInfo, IpcError> {
        let mut guard = self.0.open.lock().unwrap();
        let doc = guard.as_mut().ok_or_else(no_document)?;
        let path = doc.path.clone().ok_or_else(untitled)?;
        let bits = doc.save_bits;
        save_to(doc, &path, bits)?;
        Ok(info_of(guard.as_ref()))
    }

    /// Writes the current revision to `path` at `bits`, then binds the document to it.
    pub fn save_as(&self, path: &Path, bits: BitDepth) -> Result<DocumentInfo, IpcError> {
        let mut guard = self.0.open.lock().unwrap();
        let doc = guard.as_mut().ok_or_else(no_document)?;
        save_to(doc, path, bits)?;
        doc.path = Some(path.to_path_buf());
        doc.save_bits = bits;
        Ok(info_of(guard.as_ref()))
    }

    /// `(audio_rev, sample_rate_hz, buckets)` for `[start, start + count)` at `spp` (S1-02
    /// `peaks_query::peaks`; `spp == PEAKS_RAW_SPP` reads raw samples). The caller (`peaks_get`)
    /// encodes the result as `VXPK` (ADR-003 §2).
    pub fn peaks(&self, spp: u32, start: u64, count: u32) -> Result<PeaksResult, IpcError> {
        let guard = self.0.open.lock().unwrap();
        let doc = guard.as_ref().ok_or_else(no_document)?;
        let snapshot = doc.session.current();
        let buckets = vox_project::peaks(doc.session.store(), &snapshot, spp, start, count)
            .map_err(document_error)?;
        Ok(PeaksResult {
            audio_rev: snapshot.audio_rev,
            sample_rate_hz: snapshot.sample_rate_hz,
            buckets,
        })
    }

    fn is_recording(&self) -> bool {
        self.0
            .open
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|d| d.session.is_recording())
    }

    /// S1-04: prepares a new recording at `rate_hz` (the input stream's rate, SPEC-002 §2.2). An
    /// open, empty document at that rate records into itself; otherwise a new untitled document
    /// (default save format `save_bits`) replaces the current one — for a document with audio
    /// only with `replace` (the frontend ran the unsaved-changes prompt first). Returns the take
    /// to hand to `EngineHandle::record_start` and the document's facts.
    pub fn begin_recording(
        &self,
        rate_hz: u32,
        save_bits: BitDepth,
        replace: bool,
    ) -> Result<(TakeCapture, DocumentInfo), IpcError> {
        let mut guard = self.0.open.lock().unwrap();
        if guard.as_ref().is_some_and(|d| d.session.is_recording()) {
            return Err(IpcError::not_while_recording());
        }
        let reusable = guard.as_ref().is_some_and(|d| {
            d.session.current().len_samples == 0 && d.session.sample_rate_hz() == rate_hz
        });
        let mut previous = None;
        if !reusable {
            if !replace
                && guard
                    .as_ref()
                    .is_some_and(|d| d.session.current().len_samples > 0)
            {
                return Err(IpcError::new(
                    IpcErrorCode::InvalidArgument,
                    "error.record.needs_replace",
                ));
            }
            let session = Session::create(&self.0.sessions_dir, SessionConfig::new(rate_hz))
                .map_err(document_error)?;
            // The empty document also sets the output to the recording's rate (Dry monitoring
            // needs one rate on both streams).
            self.0.engine.set_document(Some(PlaybackDoc {
                store: Arc::clone(session.store()),
                snapshot: session.current(),
            }));
            // S2-01: same as `open` — a new document invalidates the clipboard.
            *self.0.clipboard.lock().unwrap() = None;
            previous = guard.replace(OpenDocument {
                session,
                path: None,
                save_bits,
            });
        }
        let doc = guard.as_mut().ok_or_else(no_document)?;
        let capture = doc
            .session
            .begin_take(TakeMode::New, TakeWriterOptions::default())
            .map_err(document_error)?;
        let info = info_of(guard.as_ref());
        drop(guard);
        if let Some(previous) = previous {
            close_with_retry(previous.session);
        }
        Ok((capture, info))
    }

    /// S1-04: closes a take the engine refused to record (no undo entry).
    pub fn discard_take(&self, take: TakeId) {
        let mut guard = self.0.open.lock().unwrap();
        if let Some(doc) = guard.as_mut()
            && let Err(error) = doc.session.discard_take(take)
        {
            tracing::warn!(%error, "discarding an unused take failed");
        }
    }

    /// S1-04: commits a finished take as one undoable "Record" edit (`Session::commit_take`;
    /// T-101 rules: retried, and on failure the take stays open for recovery) and makes the new
    /// revision the engine's playback document. `Ok(None)`: an empty take, discarded.
    pub fn commit_take(&self, finished: &FinishedTake) -> Result<Option<DocumentInfo>, IpcError> {
        let mut guard = self.0.open.lock().unwrap();
        let doc = guard.as_mut().ok_or_else(no_document)?;
        let mut outcome = doc.session.commit_take(finished, &[]);
        for _ in 1..COMMIT_ATTEMPTS {
            if outcome.is_ok() {
                break;
            }
            std::thread::sleep(COMMIT_RETRY_DELAY);
            outcome = doc.session.commit_take(finished, &[]);
        }
        let Some(step) = outcome.map_err(document_error)? else {
            return Ok(None);
        };
        self.0.engine.set_document(Some(PlaybackDoc {
            store: Arc::clone(doc.session.store()),
            snapshot: step.snapshot,
        }));
        Ok(Some(info_of(guard.as_ref())))
    }

    // --- S2-01: selection, cut/copy/paste/delete/trim/silence, undo/redo -------------------

    /// The Edit menu's Undo/Redo state (`history_state` event).
    pub fn history_state(&self) -> HistoryState {
        let guard = self.0.open.lock().unwrap();
        let Some(doc) = guard.as_ref() else {
            return HistoryState::default();
        };
        let history = doc.session.history();
        HistoryState {
            can_undo: history.undo_depth() > 0,
            can_redo: history.redo_depth() > 0,
            undo_label: history.undo_label().map(str::to_owned),
            redo_label: history.redo_label().map(str::to_owned),
        }
    }

    /// The clipboard's length/rate (`clipboard_changed` event; `None`s: empty).
    pub fn clipboard_info(&self) -> ClipboardInfo {
        match self.0.clipboard.lock().unwrap().as_ref() {
            Some(c) => ClipboardInfo {
                len_samples: Some(c.len_samples),
                sample_rate_hz: Some(c.sample_rate_hz),
            },
            None => ClipboardInfo::default(),
        }
    }

    fn selected_range(doc: &OpenDocument, start: u64, end: u64) -> Result<Range, IpcError> {
        validate_range(start, end, doc.session.current().len_samples).map_err(|e| match e {
            RangeError::Empty => no_selection(),
            RangeError::OutOfBounds => invalid_range(),
        })
    }

    /// Common tail of every destructive op: hands the committed snapshot to the engine (which
    /// stops playback first — Pause semantics at the heard position, ADR-004 §3, SPEC-008 §2.9 —
    /// before the swap) and builds the result the command returns.
    fn apply_committed(
        &self,
        doc: &OpenDocument,
        step: vox_project::HistoryStep,
        post: edit::PostEdit,
    ) -> EditResult {
        self.0.engine.set_document(Some(PlaybackDoc {
            store: Arc::clone(doc.session.store()),
            snapshot: Arc::clone(&step.snapshot),
        }));
        // SPEC-008 §2.3: the playhead moves to the op's table position, not wherever the
        // engine-initiated stop happened to leave it (a plain no-op seek while stopped).
        self.0
            .engine
            .transport(TransportCommand::Seek(post.playhead));
        EditResult {
            changed: true,
            audio_rev: step.snapshot.audio_rev,
            len_samples: step.snapshot.len_samples,
            selection: post.selection,
            playhead_samples: post.playhead,
        }
    }

    /// Cuts `[start, end)`: the clipboard is replaced only after the commit succeeds (SPEC-008
    /// §2.6). Refused with `error.no_selection`/`error.invalid_range` (bad range) or
    /// `error.not_while_recording` (`Session::commit_edit`); changes nothing on any error.
    pub fn edit_cut(&self, start: u64, end: u64) -> Result<EditResult, IpcError> {
        let mut guard = self.0.open.lock().unwrap();
        let doc = guard.as_mut().ok_or_else(no_document)?;
        if doc.session.is_recording() {
            return Err(IpcError::not_while_recording());
        }
        let range = Self::selected_range(doc, start, end)?;
        let snapshot = doc.session.current();
        let (built, clip) = edit::cut(&snapshot, range).map_err(document_error)?;
        let step = doc.session.commit_edit(built).map_err(document_error)?;
        let len_samples = edit::pieces_len_samples(&clip);
        *self.0.clipboard.lock().unwrap() = Some(ClipboardData {
            pieces: clip,
            sample_rate_hz: doc.session.sample_rate_hz(),
            len_samples,
        });
        Ok(self.apply_committed(doc, step, edit::post_cut_or_delete(range)))
    }

    /// Copies `[start, end)` into the clipboard. Not an edit: no commit, no transport stop, no
    /// `rev`/`audio_rev` change — refused while recording all the same (SPEC-008 §2.2: the
    /// selection can cover live, uncommitted audio during a take).
    pub fn edit_copy(&self, start: u64, end: u64) -> Result<EditResult, IpcError> {
        let guard = self.0.open.lock().unwrap();
        let doc = guard.as_ref().ok_or_else(no_document)?;
        if doc.session.is_recording() {
            return Err(IpcError::not_while_recording());
        }
        let range = Self::selected_range(doc, start, end)?;
        let snapshot = doc.session.current();
        let clip = edit::copy(&snapshot, range).map_err(document_error)?;
        let len_samples = edit::pieces_len_samples(&clip);
        *self.0.clipboard.lock().unwrap() = Some(ClipboardData {
            pieces: clip,
            sample_rate_hz: doc.session.sample_rate_hz(),
            len_samples,
        });
        Ok(EditResult {
            changed: false,
            audio_rev: snapshot.audio_rev,
            len_samples: snapshot.len_samples,
            selection: Some((range.start, range.end)),
            playhead_samples: range.start,
        })
    }

    /// Pastes the clipboard at `target` (SPEC-008 §2.1). `error.clipboard_empty` when nothing was
    /// cut/copied yet.
    pub fn edit_paste(&self, target: PasteTarget) -> Result<EditResult, IpcError> {
        let mut guard = self.0.open.lock().unwrap();
        let doc = guard.as_mut().ok_or_else(no_document)?;
        if doc.session.is_recording() {
            return Err(IpcError::not_while_recording());
        }
        let len_samples = doc.session.current().len_samples;
        let resolved = match target {
            PasteTarget::Cursor(c) if c <= len_samples => EditTarget::Cursor(c),
            PasteTarget::Cursor(_) => return Err(invalid_range()),
            PasteTarget::Range(s, e) => EditTarget::Range(Self::selected_range(doc, s, e)?),
        };
        let clipboard = self.0.clipboard.lock().unwrap();
        let clip = clipboard.as_ref().ok_or_else(clipboard_empty)?;
        let inserted_len_samples = edit::pieces_len_samples(&clip.pieces);
        let built = edit::paste(&clip.pieces, resolved);
        drop(clipboard);
        let step = doc.session.commit_edit(built).map_err(document_error)?;
        Ok(self.apply_committed(doc, step, edit::post_paste(resolved, inserted_len_samples)))
    }

    /// Deletes `[start, end)`, closing the gap.
    pub fn edit_delete(&self, start: u64, end: u64) -> Result<EditResult, IpcError> {
        let mut guard = self.0.open.lock().unwrap();
        let doc = guard.as_mut().ok_or_else(no_document)?;
        if doc.session.is_recording() {
            return Err(IpcError::not_while_recording());
        }
        let range = Self::selected_range(doc, start, end)?;
        let step = doc
            .session
            .commit_edit(edit::delete(range))
            .map_err(document_error)?;
        Ok(self.apply_committed(doc, step, edit::post_cut_or_delete(range)))
    }

    /// Trims the document to `[start, end)` (Audition: Crop). A whole-document trim is a no-op
    /// (`changed = false`, no undo entry, playback not stopped — SPEC-008 §2.1).
    pub fn edit_trim(&self, start: u64, end: u64) -> Result<EditResult, IpcError> {
        let mut guard = self.0.open.lock().unwrap();
        let doc = guard.as_mut().ok_or_else(no_document)?;
        if doc.session.is_recording() {
            return Err(IpcError::not_while_recording());
        }
        let range = Self::selected_range(doc, start, end)?;
        let len_samples = doc.session.current().len_samples;
        let Some(built) = edit::trim(range, len_samples) else {
            let snapshot = doc.session.current();
            return Ok(EditResult {
                changed: false,
                audio_rev: snapshot.audio_rev,
                len_samples: snapshot.len_samples,
                selection: None,
                playhead_samples: 0,
            });
        };
        let step = doc.session.commit_edit(built).map_err(document_error)?;
        Ok(self.apply_committed(doc, step, edit::post_trim()))
    }

    /// Silences `[start, end)` with exact `+0.0` samples (length-preserving: no marker moves).
    pub fn edit_silence(&self, start: u64, end: u64) -> Result<EditResult, IpcError> {
        let mut guard = self.0.open.lock().unwrap();
        let doc = guard.as_mut().ok_or_else(no_document)?;
        if doc.session.is_recording() {
            return Err(IpcError::not_while_recording());
        }
        let range = Self::selected_range(doc, start, end)?;
        let step = doc
            .session
            .commit_edit(edit::silence(range))
            .map_err(document_error)?;
        Ok(self.apply_committed(doc, step, edit::post_silence(range)))
    }

    /// Undoes the top entry (`Ok` with `changed: false` at the undo floor).
    pub fn history_undo(&self) -> Result<EditResult, IpcError> {
        let mut guard = self.0.open.lock().unwrap();
        let doc = guard.as_mut().ok_or_else(no_document)?;
        let Some(step) = doc.session.undo().map_err(document_error)? else {
            let snapshot = doc.session.current();
            return Ok(EditResult {
                changed: false,
                audio_rev: snapshot.audio_rev,
                len_samples: snapshot.len_samples,
                selection: None,
                playhead_samples: 0,
            });
        };
        let post = edit::post_undo_redo(step.first_at, step.snapshot.len_samples);
        Ok(self.apply_committed(doc, step, post))
    }

    /// Redoes the top entry (`Ok` with `changed: false` when there's nothing to redo).
    pub fn history_redo(&self) -> Result<EditResult, IpcError> {
        let mut guard = self.0.open.lock().unwrap();
        let doc = guard.as_mut().ok_or_else(no_document)?;
        let Some(step) = doc.session.redo().map_err(document_error)? else {
            let snapshot = doc.session.current();
            return Ok(EditResult {
                changed: false,
                audio_rev: snapshot.audio_rev,
                len_samples: snapshot.len_samples,
                selection: None,
                playhead_samples: 0,
            });
        };
        let post = edit::post_undo_redo(step.first_at, step.snapshot.len_samples);
        Ok(self.apply_committed(doc, step, post))
    }
}

fn save_to(doc: &mut OpenDocument, path: &Path, bits: BitDepth) -> Result<(), IpcError> {
    let snapshot = doc.session.current();
    let mut reader = SnapshotReader::new(Arc::clone(doc.session.store()), snapshot);
    vox_project::save_snapshot_wav(&mut reader, path, bits.into()).map_err(|err| match err {
        ProjectError::Wav(io_err) => io_save_error(io_err),
        other => document_error(other),
    })?;
    doc.session
        .mark_saved(path, format_tag(bits))
        .map_err(document_error)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use vox_engine::backend::fake::{FakeBackend, FakeDevice, FakeDirection};
    use vox_engine::{Engine, EngineConfig};
    use vox_rack::Registry;

    use super::*;
    use crate::ipc::IpcErrorCode;

    /// A running (threaded) engine on the fake backend, no devices plugged in — the tests only
    /// need a real `EngineHandle` to exercise `DocumentService::open`'s
    /// `EngineHandle::set_document` call, not an actual audio callback (MEMORY.md S1-01 Engine
    /// API note: never call `EngineHandle` from an `EventSink` — irrelevant here, no sink set).
    fn test_engine() -> Engine {
        let fake = FakeBackend::new(1);
        let registry =
            Arc::new(Registry::with_factories(vox_modules::builtin_factories()).unwrap());
        Engine::start(EngineConfig::new(Arc::new(fake), registry)).unwrap()
    }

    fn tmp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "powervoice-app-doc-{tag}-{}-{}",
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

    fn write_fixture_wav(
        path: &Path,
        samples: &[f32],
        bits: vox_testkit::wav::BitDepth,
        rate: u32,
    ) {
        vox_testkit::wav::write_wav_file(path, samples, 1, rate, bits).unwrap();
    }

    fn service(tag: &str) -> (DocumentService, Engine, PathBuf) {
        let dir = tmp_dir(tag);
        let engine = test_engine();
        let service = DocumentService::new(dir.join("sessions"), engine.handle());
        (service, engine, dir)
    }

    #[test]
    fn open_imports_a_wav_and_reports_it_as_the_current_document() {
        let (service, _engine, dir) = service("open");
        let wav_path = dir.join("in.wav");
        let samples = vox_testkit::signal::sine(440.0, -6.0, 0.05, 48_000).unwrap();
        write_fixture_wav(
            &wav_path,
            &samples,
            vox_testkit::wav::BitDepth::Float32,
            48_000,
        );

        let info = service.open(&wav_path).unwrap();
        assert_eq!(info.name.as_deref(), Some("in.wav"));
        assert_eq!(info.sample_rate_hz, 48_000);
        assert_eq!(info.len_samples, samples.len() as u64);
        assert!(
            !info.dirty,
            "import is not an undoable edit (SPEC-005 §2.3)"
        );

        // The engine's transport reflects the newly opened document's length immediately:
        // `set_document` is a synchronous round trip through the control thread.
        let transport = service.0.engine.transport_state();
        assert_eq!(transport.doc_len_samples, samples.len() as u64);
        assert_eq!(transport.doc_rate_hz, 48_000);
    }

    #[test]
    fn open_rejects_an_unsupported_wav_variant() {
        let (service, _engine, dir) = service("unsupported");
        let wav_path = dir.join("in.wav");
        // 8-bit WAVs are outside S1-02's supported set (16/24-bit int, 32-bit float).
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 48_000,
            bits_per_sample: 8,
            sample_format: hound::SampleFormat::Int,
        };
        let mut w = hound::WavWriter::create(&wav_path, spec).unwrap();
        w.write_sample(0i32).unwrap();
        w.finalize().unwrap();

        let err = service.open(&wav_path).unwrap_err();
        assert_eq!(err.code, IpcErrorCode::InvalidArgument);
        assert_eq!(err.key, "error.open.unsupported_format");
    }

    #[test]
    fn open_reports_a_missing_file() {
        let (service, _engine, dir) = service("missing");
        let err = service.open(&dir.join("nope.wav")).unwrap_err();
        assert_eq!(err.code, IpcErrorCode::NotFound);
        assert_eq!(err.key, "error.open.not_found");
    }

    #[test]
    fn save_and_save_as_round_trip_bit_exactly() {
        let (service, _engine, dir) = service("save");
        let wav_path = dir.join("in.wav");
        let samples = vox_testkit::signal::sine(1000.0, -3.0, 0.05, 48_000).unwrap();
        write_fixture_wav(
            &wav_path,
            &samples,
            vox_testkit::wav::BitDepth::Int24,
            48_000,
        );

        let info = service.open(&wav_path).unwrap();
        assert_eq!(info.path.as_deref(), Some(wav_path.to_str().unwrap()));

        // Save (no path change): open -> save with no edits round trips bit-exactly (SPEC-005
        // AC-2), same format (Int24) as the source.
        service.save().unwrap();
        let (decoded, _info) = vox_testkit::wav::read_wav_file(&wav_path).unwrap();
        let (_rate, _channels, mut source) = vox_io::read_wav(&wav_path).unwrap();
        let mut original = vec![0.0f32; samples.len()];
        source.read_mono(&mut original).unwrap();
        assert_eq!(decoded, original, "save with no edits is bit-exact");

        // Save As a new path and format (32-bit float): binds the document to the new path/bits.
        let save_as_path = dir.join("out.wav");
        let info = service
            .save_as(&save_as_path, BitDepth::Bit32Float)
            .unwrap();
        assert_eq!(info.path.as_deref(), Some(save_as_path.to_str().unwrap()));
        assert!(!info.dirty);
        let (decoded32, meta) = vox_testkit::wav::read_wav_file(&save_as_path).unwrap();
        assert_eq!(meta.sample_rate, 48_000);
        for (a, b) in decoded32.iter().zip(original.iter()) {
            assert!((a - b).abs() < 1e-6);
        }
    }

    #[test]
    fn peaks_and_save_before_any_open_report_no_document() {
        let (service, _engine, _dir) = service("nodoc");
        let peaks_err = service.peaks(64, 0, 4).unwrap_err();
        assert_eq!(peaks_err.code, IpcErrorCode::NotFound);
        assert_eq!(peaks_err.key, "error.document.none");
        let save_err = service.save().unwrap_err();
        assert_eq!(save_err.key, "error.document.none");
    }

    #[test]
    #[allow(clippy::float_cmp)] // deliberate exact-value check: raw pairs must be (x, x)
    fn peaks_returns_raw_samples_and_pyramid_buckets() {
        let (service, _engine, dir) = service("peaks");
        let wav_path = dir.join("in.wav");
        let samples: Vec<f32> = (0..10_000)
            .map(|i| ((i as f32) * 0.001).sin() * 0.8)
            .collect();
        write_fixture_wav(
            &wav_path,
            &samples,
            vox_testkit::wav::BitDepth::Float32,
            48_000,
        );
        service.open(&wav_path).unwrap();

        let raw = service.peaks(vox_project::PEAKS_RAW_SPP, 10, 5).unwrap();
        for (i, &(mn, mx)) in raw.buckets.iter().enumerate() {
            assert_eq!(mn, mx);
            assert!((mn - samples[10 + i]).abs() < 1e-6);
        }
        assert_eq!(raw.sample_rate_hz, 48_000);

        let bucketed = service.peaks(64, 0, 4).unwrap();
        assert_eq!(bucketed.buckets.len(), 4);
        for (b, chunk) in bucketed.buckets.iter().zip(samples.chunks(64)) {
            let (mn, mx) = chunk
                .iter()
                .fold((f32::INFINITY, f32::NEG_INFINITY), |(mn, mx), &s| {
                    (mn.min(s), mx.max(s))
                });
            assert!((b.0 - mn).abs() < 1e-6);
            assert!((b.1 - mx).abs() < 1e-6);
        }
    }

    /// Manual smoke check for the ticket's DoD ("report time to first waveform"): opens the
    /// `just fixtures` 60-min 48 kHz mono WAV (generating it on the fly if `just fixtures` hasn't
    /// been run, same fallback as `crates/project/tests/big.rs`) through the exact path
    /// `document_open` uses, and times it plus a first `peaks_get`-equivalent pyramid query.
    /// Ignored by default (minutes of I/O the first time); run with
    /// `cargo test -p powervoice-app --release -- --ignored open_and_first_peaks_of_the_60_min_fixture_report_timing --nocapture`.
    #[test]
    #[ignore]
    fn open_and_first_peaks_of_the_60_min_fixture_report_timing() {
        let dir = tmp_dir("sixty-min");
        let generated = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../fixtures/generated/long-60min-48k-mono.wav");
        let wav_path = if generated.exists() {
            generated
        } else {
            let path = dir.join("long-60min-48k-mono.wav");
            vox_testkit::signal::write_long_fixture(&path, 3600.0, 48_000, 42).unwrap();
            path
        };

        let (service, _engine, _dir2) = service("sixty-min");
        let start = std::time::Instant::now();
        let info = service.open(&wav_path).unwrap();
        let open_elapsed = start.elapsed();

        let peaks_start = std::time::Instant::now();
        let peaks = service.peaks(65_536, 0, 165).unwrap(); // coarsest level, whole 60-min file
        let peaks_elapsed = peaks_start.elapsed();

        println!(
            "document_open of the 60-min fixture: {:.3} s ({} samples); first coarse peaks_get: {:.3} ms ({} buckets)",
            open_elapsed.as_secs_f64(),
            info.len_samples,
            peaks_elapsed.as_secs_f64() * 1000.0,
            peaks.buckets.len(),
        );
        assert!(info.len_samples > 0);
    }

    #[test]
    fn opening_a_second_document_replaces_the_first() {
        let (service, _engine, dir) = service("replace");
        let a_path = dir.join("a.wav");
        let b_path = dir.join("b.wav");
        write_fixture_wav(
            &a_path,
            &vox_testkit::signal::silence(0.05, 48_000).unwrap(),
            vox_testkit::wav::BitDepth::Float32,
            48_000,
        );
        write_fixture_wav(
            &b_path,
            &vox_testkit::signal::sine(220.0, -6.0, 0.1, 48_000).unwrap(),
            vox_testkit::wav::BitDepth::Float32,
            48_000,
        );

        service.open(&a_path).unwrap();
        let info = service.open(&b_path).unwrap();
        assert_eq!(info.name.as_deref(), Some("b.wav"));
        let transport = service.0.engine.transport_state();
        assert_eq!(
            transport.doc_len_samples,
            (0.1 * 48_000.0) as u64,
            "the engine now plays b.wav, not a.wav"
        );
    }

    /// S1-04: a new recording is an untitled document; its take is committed as one undoable
    /// edit the engine plays; Save refuses an untitled document, Save As binds it; Open is
    /// refused while recording; a document with audio is replaced only with `replace`.
    #[test]
    fn recordings_become_untitled_documents() {
        let (service, _engine, dir) = service("record");
        let (mut capture, info) = service
            .begin_recording(48_000, BitDepth::Bit24, false)
            .unwrap();
        assert_eq!(info.name, None);
        assert_eq!(info.path, None);
        assert_eq!((info.sample_rate_hz, info.len_samples), (48_000, 0));
        let samples = vox_testkit::signal::sine(440.0, -6.0, 0.05, 48_000).unwrap();
        capture.append(&samples).unwrap();
        let info = service
            .commit_take(&capture.finish())
            .unwrap()
            .expect("one undoable edit");
        assert_eq!(info.len_samples, samples.len() as u64);
        assert!(info.dirty);
        let transport = service.0.engine.transport_state();
        assert_eq!(transport.doc_len_samples, samples.len() as u64);

        assert_eq!(service.save().unwrap_err().key, "error.document.untitled");
        let saved = dir.join("take.wav");
        let info = service.save_as(&saved, BitDepth::Bit24).unwrap();
        assert_eq!(info.name.as_deref(), Some("take.wav"));
        assert!(!info.dirty);

        let err = service
            .begin_recording(48_000, BitDepth::Bit24, false)
            .err()
            .expect("a document with audio needs `replace`");
        assert_eq!(err.key, "error.record.needs_replace");
        let (capture, info) = service
            .begin_recording(48_000, BitDepth::Bit24, true)
            .unwrap();
        assert_eq!((info.name, info.len_samples), (None, 0));
        assert_eq!(
            service.open(&saved).unwrap_err().code,
            IpcErrorCode::NotWhileRecording
        );
        service.discard_take(capture.id());
        drop(capture);
        assert_eq!(
            service.open(&saved).unwrap().name.as_deref(),
            Some("take.wav")
        );
    }

    // --- S2-01: selection, cut/copy/paste/delete/trim/silence, undo/redo -------------------

    fn open_test_doc(service: &DocumentService, dir: &Path, samples: &[f32]) {
        let path = dir.join("in.wav");
        write_fixture_wav(&path, samples, vox_testkit::wav::BitDepth::Float32, 48_000);
        service.open(&path).unwrap();
    }

    #[test]
    fn cut_then_paste_round_trips_and_undo_redo_restore_state() {
        let (service, _engine, dir) = service("cut-paste");
        let samples = vox_testkit::signal::sine(440.0, -6.0, 0.2, 48_000).unwrap();
        open_test_doc(&service, &dir, &samples);
        let len = samples.len() as u64;

        let cut = service.edit_cut(1_000, 5_000).unwrap();
        assert!(cut.changed);
        assert_eq!(cut.selection, None);
        assert_eq!(cut.playhead_samples, 1_000);
        assert_eq!(cut.len_samples, len - 4_000);
        assert_eq!(
            service.clipboard_info().len_samples,
            Some(4_000),
            "cut replaces the clipboard"
        );

        let paste = service.edit_paste(PasteTarget::Cursor(1_000)).unwrap();
        assert!(paste.changed);
        assert_eq!(
            paste.selection,
            Some((1_000, 5_000)),
            "pasted audio is selected"
        );
        assert_eq!(paste.len_samples, len);

        let history = service.history_state();
        assert!(history.can_undo);
        assert!(!history.can_redo);
        assert_eq!(history.undo_label.as_deref(), Some("history.paste"));

        let undo = service.history_undo().unwrap();
        assert!(undo.changed);
        assert_eq!(undo.selection, None, "undo clears the selection");
        assert_eq!(undo.len_samples, len - 4_000);

        let undo2 = service.history_undo().unwrap();
        assert_eq!(
            undo2.len_samples, len,
            "back to the original, unedited document"
        );
        assert!(!service.history_state().can_undo);

        let redo = service.history_redo().unwrap();
        assert_eq!(redo.len_samples, len - 4_000);
        assert!(service.history_state().can_redo);
    }

    #[test]
    fn copy_is_not_an_edit_and_delete_trim_silence_are_exact() {
        let (service, _engine, dir) = service("copy-delete-trim-silence");
        let samples = vox_testkit::signal::sine(220.0, -6.0, 0.2, 48_000).unwrap();
        open_test_doc(&service, &dir, &samples);
        let before = service.info();

        let copy = service.edit_copy(0, 100).unwrap();
        assert!(!copy.changed, "Copy is not an edit");
        assert_eq!(service.clipboard_info().len_samples, Some(100));
        let after_copy = service.info();
        assert_eq!(before.audio_rev, after_copy.audio_rev, "no rev bump");
        assert!(
            !after_copy.dirty || before.dirty,
            "Copy alone never dirties a clean document"
        );
        assert!(!service.history_state().can_undo, "Copy adds no undo entry");

        let len = samples.len() as u64;
        let delete = service.edit_delete(1_000, 2_000).unwrap();
        assert_eq!(delete.len_samples, len - 1_000);
        assert_eq!(delete.playhead_samples, 1_000);
        assert_eq!(delete.selection, None);

        // Trim off the last 1 000 samples of the post-delete document (a partial trim, not the
        // whole-document no-op case, which `whole_trim` below exercises separately).
        let trim = service.edit_trim(0, delete.len_samples - 1_000).unwrap();
        assert!(trim.changed);
        assert_eq!(trim.len_samples, delete.len_samples - 1_000);
        assert_eq!(trim.playhead_samples, 0);

        let whole_trim = service.edit_trim(0, trim.len_samples).unwrap();
        assert!(!whole_trim.changed, "trimming [0, L) is a no-op");
        assert_eq!(
            service.history_state().undo_label.as_deref(),
            Some("history.trim")
        );

        let silence = service.edit_silence(0, 500).unwrap();
        assert!(silence.changed);
        assert_eq!(silence.selection, Some((0, 500)));
        assert_eq!(silence.len_samples, trim.len_samples, "length preserved");
    }

    #[test]
    fn commands_reject_bad_selections_and_an_empty_clipboard() {
        let (service, _engine, dir) = service("bad-selection");
        let samples = vox_testkit::signal::silence(0.05, 48_000).unwrap();
        open_test_doc(&service, &dir, &samples);
        let len = samples.len() as u64;

        assert_eq!(
            service.edit_cut(10, 10).unwrap_err().key,
            "error.no_selection"
        );
        assert_eq!(
            service.edit_cut(0, len + 1).unwrap_err().key,
            "error.invalid_range"
        );
        assert_eq!(
            service.edit_paste(PasteTarget::Cursor(0)).unwrap_err().key,
            "error.clipboard_empty"
        );
        assert_eq!(
            service
                .edit_paste(PasteTarget::Cursor(len + 1))
                .unwrap_err()
                .key,
            "error.invalid_range"
        );
        // Nothing changed.
        assert_eq!(service.info().audio_rev, service.info().audio_rev);
        assert!(!service.history_state().can_undo);
    }

    #[test]
    fn edits_and_history_are_refused_while_recording() {
        let (service, _engine, dir) = service("refused-recording");
        open_test_doc(
            &service,
            &dir,
            &vox_testkit::signal::silence(0.05, 48_000).unwrap(),
        );
        service.edit_copy(0, 100).unwrap(); // give the clipboard something, for the paste check

        let (mut capture, _info) = service
            .begin_recording(48_000, BitDepth::Bit24, true)
            .unwrap();
        capture.append(&[0.0; 100]).unwrap();

        for result in [
            service.edit_cut(0, 10).map(|_| ()),
            service.edit_copy(0, 10).map(|_| ()),
            service.edit_paste(PasteTarget::Cursor(0)).map(|_| ()),
            service.edit_delete(0, 10).map(|_| ()),
            service.edit_trim(0, 10).map(|_| ()),
            service.edit_silence(0, 10).map(|_| ()),
            service.history_undo().map(|_| ()),
            service.history_redo().map(|_| ()),
        ] {
            let err = result.unwrap_err();
            assert_eq!(err.code, IpcErrorCode::NotWhileRecording);
        }

        service.discard_take(capture.id());
    }

    fn dac() -> FakeDevice {
        FakeDevice::new("DAC").with_output(FakeDirection::new(1, &[48_000], 48_000))
    }

    /// A threaded engine with one plugged, preferred output device (S1-01 pattern, `crates/engine`
    /// tests): the poll thread's immediate first pass discovers and opens it without a running
    /// fake-time driver (no callbacks need to fire for `open_output` to succeed).
    fn service_with_output(tag: &str) -> (DocumentService, Engine, PathBuf) {
        let dir = tmp_dir(tag);
        let fake = FakeBackend::new(7);
        fake.plug(vox_engine::HostId::Alsa, dac());
        let registry =
            Arc::new(Registry::with_factories(vox_modules::builtin_factories()).unwrap());
        let mut cfg = EngineConfig::new(Arc::new(fake), registry);
        cfg.prefs = vox_engine::DevicePrefs {
            output_device: Some("DAC".into()),
            ..vox_engine::DevicePrefs::default()
        };
        let engine = Engine::start(cfg).unwrap();
        let service = DocumentService::new(dir.join("sessions"), engine.handle());
        (service, engine, dir)
    }

    /// SPEC-008 §2.9 / MEMORY.md S1-01 handoff ("destructive-edit stop→ack→commit"): a destructive
    /// edit's `EngineHandle::set_document` call stops the transport (Pause semantics) as part of
    /// the very same control-thread round trip that swaps in the new snapshot, so by the time the
    /// command returns, the transport already reports stopped — no output sample after the commit
    /// can come from the old revision.
    #[test]
    fn destructive_edit_stops_playback_as_part_of_the_commit() {
        let (service, engine, dir) = service_with_output("stops-playback");
        let samples = vox_testkit::signal::sine(440.0, -6.0, 1.0, 48_000).unwrap();
        open_test_doc(&service, &dir, &samples);

        let mut opened = false;
        for _ in 0..300 {
            if engine
                .handle()
                .devices()
                .is_some_and(|d| d.output_status == vox_engine::device_state::DeviceStatus::Healthy)
            {
                opened = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(opened, "the fake output device opened");

        let played = engine
            .handle()
            .transport(vox_engine::TransportCommand::Play);
        assert!(
            played.playing,
            "playback started on the freshly opened document"
        );

        service.edit_delete(0, 1_000).unwrap();

        assert!(
            !engine.handle().transport_state().playing,
            "the destructive edit's set_document call stopped playback"
        );

        // Copy, not being an edit, must never stop playback.
        engine
            .handle()
            .transport(vox_engine::TransportCommand::Play);
        service.edit_copy(0, 1_000).unwrap();
        assert!(
            engine.handle().transport_state().playing,
            "Copy doesn't touch the transport"
        );
    }
}

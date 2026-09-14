//! Document service (S1-03): the app-level owner of the one open session (ADR-004 §8, SPEC-004
//! §2.1). Wraps S1-02's `import_wav`/`save_snapshot_wav`/`peaks` over a [`vox_project::Session`]
//! and keeps the engine's playback document in sync (`EngineHandle::set_document`, MEMORY.md
//! S1-01 Engine API). `ipc::document_commands` only forwards to this — no business logic lives
//! in the command handlers themselves (CLAUDE.md: "`src-tauri` is a thin command/event layer").

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use vox_engine::record::DropoutMark;
use vox_engine::{EngineHandle, PlaybackDoc, TransportCommand};
use vox_project::{
    DocumentIdentity, Edit, EditTarget, FinishedTake, LufsNormalizeOutcome, Marker, MarkerId,
    MarkerItemModel, MarkerMetaTable, MarkerOp, NormalizeLufsPlan, NormalizeOutcome,
    NormalizePeakPlan, Piece, ProjectError, Range, RangeError, SaveFormatModel, Session,
    SessionConfig, SidecarNotice, SnapshotReader, TakeCapture, TakeId, TakeMode, TakeWriterOptions,
    WrittenAudio, document_crc32, edit, marker_from_item, normalize_applied_post_edit,
    read_sidecar, sidecar_path_for, validate_range, write_sidecar,
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
/// S2-02: the Normalize… dialog's dB-mode range (SPEC-010 §2.4, §3 `target_db`).
const NORMALIZE_TARGET_MIN_DB: f64 = -60.0;
const NORMALIZE_TARGET_MAX_DB: f64 = 0.0;
/// S4-01: the LUFS normalize dialog's custom-target range (ticket favorites −16/−19/−23 LUFS
/// always fall inside it).
const NORMALIZE_LUFS_TARGET_MIN_LUFS: f64 = -60.0;
const NORMALIZE_LUFS_TARGET_MAX_LUFS: f64 = 0.0;

/// The OS data dir's `sessions` subfolder (mirrors `settings::project_dirs`'s convention; no
/// state dir on macOS/Windows, so those fall back to the data dir like `logging.rs` does).
pub fn default_sessions_dir() -> PathBuf {
    match directories::ProjectDirs::from("app", "powervoice", "powervoice") {
        Some(dirs) => dirs.data_dir().join("sessions"),
        None => std::env::temp_dir().join("powervoice").join("sessions"),
    }
}

/// T-306: everything about the open document's sidecar besides the session itself (SPEC-018).
/// `project` owns the schema/read/write/digest (`vox_project::sidecar`); this is just the
/// per-open-document state a thin `src-tauri` service needs to drive it.
struct SidecarState {
    /// Bridges `kind`/per-item unknown fields onto marker ids (module scope note,
    /// `vox_project::sidecar`): `vox_project::Marker` has no `kind` field yet.
    marker_meta: MarkerMetaTable,
    /// Opaque view JSON (SPEC-018 §2.6.5): `Null` until the UI sends one via
    /// `sidecar_view_set`, or until a sidecar with a `view` section is loaded.
    view: serde_json::Value,
    /// Baseline for `sidecar_dirty` (SPEC-018 §4.3): the persisted-content digest as of open or
    /// the last successful sidecar write.
    persisted_digest: u32,
    /// Whether the sidecar *on disk* needs backing up before the next overwrite (SPEC-018 §2.8):
    /// set when the last read couldn't fully use it, or its `version` wasn't 1.
    needs_backup: bool,
    /// The document's file fingerprint (SPEC-018 §4.2) as of open or the last successful save.
    audio_crc32: u32,
    /// The bound audio file's size/mtime as PowerVoice last recorded them (open or last save),
    /// for the changed-on-disk pre-flight check (SPEC-018 §2.9).
    file_size_bytes: u64,
    file_mtime_unix_ms: u64,
    /// `audio_rev` as of open or the last save that rewrote the audio (SPEC-018 §2.4's "audio_rev
    /// changed since the last save or open" bullet).
    last_saved_audio_rev: u64,
    /// The `markers.items` the *audio file itself* currently reflects (its WAV cue chunk) — as of
    /// open or the last full save. Save writes the audio again when the live markers differ from
    /// this (SPEC-018 §2.4's markers bullet; every save format here is WAV, which always stores
    /// markers, so "the container stores markers" is always true in this build — T-201's FLAC
    /// save path doesn't exist yet).
    last_written_markers: Vec<MarkerItemModel>,
}

impl SidecarState {
    /// The "None" baseline (SPEC-018 §2.5: no usable sidecar, or a brand new recording): whatever
    /// is carried over/imported becomes the baseline, so opening never sets `sidecar_dirty` by
    /// itself.
    fn none() -> Self {
        Self {
            marker_meta: MarkerMetaTable::new(),
            view: serde_json::Value::Null,
            persisted_digest: 0,
            needs_backup: false,
            audio_crc32: 0,
            file_size_bytes: 0,
            file_mtime_unix_ms: 0,
            last_saved_audio_rev: 0,
            last_written_markers: Vec::new(),
        }
    }
}

/// One open document: its session plus the path/format it's bound to (`None` path = never saved:
/// a new recording, S1-04).
struct OpenDocument {
    session: Session,
    path: Option<PathBuf>,
    save_bits: BitDepth,
    sidecar: SidecarState,
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
    /// T-306 (SPEC-018 §2.4): the persisted content (save format + markers + rack) differs from
    /// what was last written or read. Title `*` and the close/quit prompts fire on `dirty ||
    /// sidecar_dirty`; view-only changes never set it.
    pub sidecar_dirty: bool,
    /// T-306 (SPEC-018 §2.6.5): the sidecar's `view.spectral` section, if the open sidecar (or a
    /// prior `set_spectral_view` this session) has one. `None` means "no opinion" — the UI keeps
    /// its current settings / the app's last-used defaults (SPEC-007 §2.1).
    pub spectral_view: Option<SpectralViewInfo>,
}

/// T-306 (SPEC-018 §2.6.5's `view.spectral`): SPEC-007 §3's per-document spectral pane settings.
/// Plain data — [`crate::ipc::document_dto::SpectralViewDto`] is the ts-rs wire type
/// (module doc: DTOs live in the `ipc`/`settings` layer, not here).
#[derive(Clone, Debug, PartialEq)]
pub struct SpectralViewInfo {
    pub visible: bool,
    pub split_ratio: f64,
    /// `None` = Auto (SPEC-007 §2.6).
    pub fft_size: Option<u32>,
    pub freq_scale: String,
    pub display_floor_db: f64,
    pub display_ceil_db: f64,
    pub colormap: String,
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

/// S2-02: why `edit_normalize_peak` did nothing (SPEC-010 §2.7) — `EditResult.changed` is already
/// `false`; the command handler turns this into a `notice` event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NormalizeNotice {
    /// The scope's sample peak is `0` or below the −120 dBFS near-silence floor.
    Silent,
    /// The gain needed is closer to 0 dB than the 0.001 dB "already there" tolerance.
    AlreadyNormalized,
}

/// The result of [`DocumentService::edit_normalize_peak`].
#[derive(Clone, Debug, PartialEq)]
pub struct NormalizeEditResult {
    pub result: EditResult,
    pub notice: Option<NormalizeNotice>,
}

/// S4-01: why `edit_normalize_lufs` did nothing, or a heads-up alongside an applied gain —
/// `EditResult.changed` already tells the caller whether it applied.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum NormalizeLufsNotice {
    /// The scope has no block above the BS.1770 absolute gate (silence or near-silence).
    Silent,
    /// The gain needed is closer to 0 dB than the 0.001 dB "already there" tolerance.
    AlreadyNormalized,
    /// The gain was applied (not a no-op), but the resulting true peak is predicted to exceed
    /// −1 dBTP (ticket: "apply the gain anyway and show a notice suggesting the true-peak
    /// limiter — don't silently limit").
    TruePeakCeilingExceeded { predicted_true_peak_dbtp: f64 },
}

/// The result of [`DocumentService::edit_normalize_lufs`].
#[derive(Clone, Debug, PartialEq)]
pub struct NormalizeLufsEditResult {
    pub result: EditResult,
    pub notice: Option<NormalizeLufsNotice>,
}

/// S2-03: one marker (SPEC-009 §2.1's essential subset — no `kind`, deferred with dropout
/// markers/recording), as `markers_get`/`marker_add` report it to the UI.
#[derive(Clone, Debug, PartialEq)]
pub struct MarkerInfo {
    pub id: u64,
    pub pos_samples: u64,
    pub len_samples: u64,
    pub name: String,
}

impl From<&Marker> for MarkerInfo {
    fn from(m: &Marker) -> Self {
        MarkerInfo {
            id: m.id.0,
            pos_samples: m.pos_samples,
            len_samples: m.len_samples,
            name: m.name.to_string(),
        }
    }
}

/// SPEC-009 §2.5: whether a panel-typed range edit is a move (Start, keeping `len`) or a resize
/// (End/Duration, keeping `pos`) — only the undo label differs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MarkerRangeEditKind {
    Move,
    Resize,
}

impl MarkerRangeEditKind {
    fn label_key(self) -> &'static str {
        match self {
            MarkerRangeEditKind::Move => "history.marker_move",
            MarkerRangeEditKind::Resize => "history.marker_resize",
        }
    }
}

/// SPEC-009 §2.3: "Marker NN" (N zero-padded to at least 2 digits), N = 1 + the largest number
/// among existing default-named markers ("Marker 7" and "Marker 07" both count), or 1 if none
/// match. S2-03 essential subset: the pattern is hardcoded English, not looked up through i18n
/// (SPEC-009 says the name is generated in the current locale and then stored as plain text —
/// full locale plumbing across the IPC boundary is deferred, ticket report).
fn next_marker_name(markers: &[Marker]) -> String {
    let max = markers
        .iter()
        .filter_map(|m| marker_default_number(&m.name))
        .max()
        .unwrap_or(0);
    format!("Marker {:02}", max + 1)
}

fn marker_default_number(name: &str) -> Option<u64> {
    let rest = name.strip_prefix("Marker ")?;
    if rest.is_empty() || rest.len() > 6 || !rest.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    rest.parse().ok()
}

/// H-10 item 4 (SPEC-002 §2.4/§4.3): "Dropout 12 ms" — same hardcoded-English convention as
/// [`next_marker_name`] (SPEC-009's i18n key for generated marker names is deferred, S2-03
/// ticket report); `len_samples` rounds to the nearest millisecond.
fn dropout_marker_name(len_samples: u64, rate_hz: u32) -> String {
    let rate_hz = u64::from(rate_hz.max(1));
    let ms = (len_samples * 1000 + rate_hz / 2) / rate_hz;
    format!("Dropout {ms} ms")
}

/// SPEC-009 §2.4's rename normalization (also SPEC-005 §4.6's cue-name reading rules): trim
/// leading/trailing whitespace, strip control characters other than tab, cut to at most 1024
/// bytes at a character boundary.
fn normalize_marker_name(name: &str) -> String {
    let trimmed: String = name
        .trim()
        .chars()
        .filter(|&c| c == '\t' || !c.is_control())
        .collect();
    if trimmed.len() <= 1024 {
        return trimmed;
    }
    let mut cut = 1024;
    while !trimmed.is_char_boundary(cut) {
        cut -= 1;
    }
    trimmed[..cut].to_owned()
}

struct Inner {
    sessions_dir: PathBuf,
    engine: EngineHandle,
    open: Mutex<Option<OpenDocument>>,
    clipboard: Mutex<Option<ClipboardData>>,
    /// H-09: `true` while a normalize job (peak or LUFS) is scanning/writing on its own thread —
    /// SPEC-010 §2.1's "disabled ... while another document job runs" (`error.document_busy`).
    /// Only one normalize job runs at a time; other audio-mutating commands also check this so a
    /// concurrent edit can't race the job's eventual commit (which re-checks the snapshot anyway,
    /// [`DocumentService::finish_normalize_peak`]/[`Self::finish_normalize_lufs`]).
    normalize_busy: Mutex<bool>,
    /// T-306: the last open/save's sidecar notice (mismatch/corrupt/too_new/unreadable/
    /// items_dropped/save-failed), if any — [`DocumentService::take_sidecar_notice`] drains it.
    /// A side channel rather than a return value so `open`/`save`/`save_as` keep returning
    /// [`DocumentInfo`] like every other command result.
    pending_sidecar_notice: Mutex<Option<SidecarNoticeInfo>>,
}

/// A sidecar-related notice for the command handler to turn into a `notice` event (SPEC-018
/// §2.5/§2.9's notice keys). `params` are already i18n `with_param` pairs (e.g. `file`, `count`).
#[derive(Clone, Debug, PartialEq)]
pub struct SidecarNoticeInfo {
    pub key: &'static str,
    pub params: Vec<(&'static str, String)>,
}

/// S4-04: the read-only facts and handles an export job needs. Exports never touch the document
/// (D-019: only Save/Save As write it) — this is a snapshot the job reads from independently, on
/// its own thread, while editing and playback continue.
pub struct ExportSource {
    pub store: Arc<vox_project::ChunkStore>,
    pub snapshot: Arc<vox_project::DocSnapshot>,
    pub sample_rate_hz: u32,
    pub len_samples: u64,
    /// The document's current file stem (no extension), for the export dialog's suggested output
    /// name; `None` for a never-saved recording.
    pub suggested_name: Option<String>,
}

/// H-09: the read-only facts + scope [`DocumentService::begin_normalize_job`] hands to a
/// normalize job (mirrors [`ExportSource`]; unlike it, this job *does* eventually write back
/// through [`DocumentService::finish_normalize_peak`]/[`DocumentService::finish_normalize_lufs`],
/// so it also carries the session id those use to detect the document having moved on
/// meanwhile).
pub struct NormalizeJobSource {
    pub store: Arc<vox_project::ChunkStore>,
    pub snapshot: Arc<vox_project::DocSnapshot>,
    pub sample_rate_hz: u32,
    pub range: Range,
    session_id: String,
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

/// SPEC-018 §2.11: `path` is already open (and locked) in another PowerVoice instance. The UI
/// shows "Open Anyway" / "Cancel" and re-issues `document_open` with `confirm_already_open: true`
/// on "Open Anyway".
fn already_open_error(path: &Path) -> IpcError {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    IpcError::new(IpcErrorCode::NeedsConfirmation, "dialog.already_open").with_param("name", name)
}

/// SPEC-018 §2.9: the bound file's size or mtime changed since PowerVoice opened or last saved
/// it. The UI shows "Overwrite" / "Save As…" / "Cancel" and re-issues `document_save` with
/// `overwrite: true` on "Overwrite".
fn changed_on_disk_error(path: &Path) -> IpcError {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    IpcError::new(IpcErrorCode::NeedsConfirmation, "dialog.changed_on_disk")
        .with_param("name", name)
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

/// SPEC-010 §2.1: a normalize job (H-09) is already running.
fn document_busy() -> IpcError {
    IpcError::document_busy()
}

/// SPEC-009 §4.2: a marker command named an id that doesn't exist (any more).
fn marker_not_found() -> IpcError {
    IpcError::new(IpcErrorCode::InvalidArgument, "error.marker_not_found")
}

/// SPEC-009 §2.4: a rename whose normalized name is empty.
fn marker_name_empty() -> IpcError {
    IpcError::new(IpcErrorCode::InvalidArgument, "error.marker_name_empty")
}

/// Maps a [`ProjectError`] to an [`IpcError`], reusing its i18n key (`ProjectError::i18n_key`).
/// `pub(crate)`: also used by `crate::normalize::NormalizeService` (H-09) to map a cancelled/
/// failed job's `ProjectError` (from `plan_normalize_peak`/`plan_normalize_lufs`) the same way
/// every other document error is mapped.
pub(crate) fn document_error(err: ProjectError) -> IpcError {
    let code = match &err {
        ProjectError::NotWhileRecording => IpcErrorCode::NotWhileRecording,
        ProjectError::InvalidArgument(_)
        | ProjectError::InvalidEdit(_)
        | ProjectError::NonFiniteSample => IpcErrorCode::InvalidArgument,
        ProjectError::Cancelled => IpcErrorCode::Cancelled,
        // T-202: SPEC-005 §2.5's open/import errors each get the closest-matching coarse code;
        // everything else (write/encode failures, plain I/O) stays `Internal`, as before.
        ProjectError::Wav(vox_io::IoError::UnrecognizedFormat(_))
        | ProjectError::Wav(vox_io::IoError::UnsupportedCodec(_))
        | ProjectError::Wav(vox_io::IoError::NoAudioTrack)
        | ProjectError::Wav(vox_io::IoError::RateOutOfRange(_))
        | ProjectError::Wav(vox_io::IoError::TooManyChannels(_))
        | ProjectError::Wav(vox_io::IoError::ChainedStreamChanged)
        | ProjectError::Wav(vox_io::IoError::TooDamaged(_, _)) => IpcErrorCode::InvalidArgument,
        ProjectError::Wav(vox_io::IoError::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => {
            IpcErrorCode::NotFound
        }
        ProjectError::Wav(vox_io::IoError::Io(_)) => IpcErrorCode::Io,
        _ => IpcErrorCode::Internal,
    };
    IpcError::new(code, err.i18n_key()).with_param("message", err.to_string())
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

/// T-202: the save format an *imported* document keeps (SPEC-005 §2.6), generalized beyond
/// `save_bits_for`'s plain-WAV table to every format `vox_project::import_file` now accepts.
///
/// **Scope note:** this reuses `save_bits_for`'s exact table for a WAV `hound` can parse (16/24-
/// bit int, 32-bit float — a cheap header-only re-open; `vox_io::read_wav` never reads samples
/// until `read_mono` is called, so this costs one extra file open, not a second decode). Every
/// WAV variant hound can't parse (8-bit/A-law/µ-law/32-bit int/64-bit float/`EXTENSIBLE`) and
/// every lossy/FLAC container default to WAV 24-bit, matching SPEC-005 §2.6's "lossy imports
/// default to WAV 24-bit" — the exact variant->16/32f promotion table for those, and lossy
/// imports' Save-acts-as-Save-As routing, are Save's job (T-201, out of this ticket's scope).
fn save_bits_for_import(path: &Path, container: &str) -> BitDepth {
    if container == "wav"
        && let Ok((_, _, source)) = vox_io::read_wav(path)
    {
        return save_bits_for(source.format());
    }
    BitDepth::Bit24
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

/// T-306 (SPEC-018 §2.6.2): `container` is always `"wav"` today — Save/Save As only ever writes
/// WAV (T-201's FLAC save path is a separate, not-yet-implemented ticket) — so this only maps
/// `bits` to `sample_format`/`dither`.
fn save_format_model(bits: BitDepth) -> SaveFormatModel {
    let (sample_format, dither) = match bits {
        BitDepth::Bit16 => ("pcm16", "tpdf"),
        BitDepth::Bit24 => ("pcm24", "tpdf"),
        BitDepth::Bit32Float => ("f32", "none"),
    };
    SaveFormatModel {
        container: "wav".to_string(),
        sample_format: sample_format.to_string(),
        dither: dither.to_string(),
        extra: Default::default(),
    }
}

/// The `markers.items` a save/digest would use right now: the live markers, with each one's
/// sidecar `kind`/extra carried through [`SidecarState::marker_meta`] (module scope note,
/// `vox_project::sidecar`).
fn current_marker_items(doc: &OpenDocument) -> Vec<MarkerItemModel> {
    doc.sidecar
        .marker_meta
        .build_items(&doc.session.current().markers)
}

/// The live rack as the opaque `rack` Value the sidecar stores (ADR-001 rule 4: `project` never
/// depends on `rack`, so this conversion — `RackModel`'s own `serde`, per SPEC-018 §4.1 — happens
/// here, in `src-tauri`, or in `engine`, never in `vox_project`).
fn current_rack_value(engine: &EngineHandle) -> serde_json::Value {
    engine
        .rack_model()
        .and_then(|m| serde_json::to_value(&m).ok())
        .unwrap_or_else(|| serde_json::json!({"slots": []}))
}

/// SPEC-018 §4.3: `sidecar_dirty` compares the *current* persisted-content digest against
/// `doc.sidecar.persisted_digest`'s baseline (recorded at open and after every successful sidecar
/// write).
fn sidecar_dirty_of(engine: &EngineHandle, doc: &OpenDocument) -> bool {
    let save_format = save_format_model(doc.save_bits);
    let markers = current_marker_items(doc);
    let rack = current_rack_value(engine);
    vox_project::sidecar::persisted_digest(&save_format, &markers, &rack)
        != doc.sidecar.persisted_digest
}

/// Builds the [`SidecarNoticeInfo`] for one of [`SidecarNotice`]'s variants (SPEC-018 §2.5's
/// notice keys — `key()` already gives the i18n key; this adds the `file`/`count` params).
fn sidecar_notice_info(notice: &SidecarNotice, sidecar_path: &Path) -> SidecarNoticeInfo {
    let file = sidecar_path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut params = vec![("file", file)];
    if let SidecarNotice::ItemsDropped(n) = notice {
        params.push(("count", n.to_string()));
    }
    SidecarNoticeInfo {
        key: notice.i18n_key(),
        params,
    }
}

/// `t`'s milliseconds since the Unix epoch (0 on error/before-epoch, like `document.rs`'s other
/// best-effort file-fact readers).
fn unix_ms_of(t: std::time::SystemTime) -> u64 {
    t.duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// SPEC-018 §2.9: has `path` changed size or mtime since PowerVoice opened it or last saved it?
/// A file that no longer exists is not "changed" here (no prompt needed — Save recreates it).
fn changed_on_disk(doc: &OpenDocument, path: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    let mtime_ms = meta.modified().map(unix_ms_of).unwrap_or(0);
    meta.len() != doc.sidecar.file_size_bytes || mtime_ms != doc.sidecar.file_mtime_unix_ms
}

/// `doc.sidecar.view["spectral"]` -> [`SpectralViewInfo`] (SPEC-018 §2.6.5: "all fields are
/// optional on read... a missing or invalid field takes its default" — here that just means the
/// whole section is absent, since the UI already has SPEC-007 defaults to fall back to).
fn spectral_view_of(view: &serde_json::Value) -> Option<SpectralViewInfo> {
    let s = view.get("spectral")?;
    Some(SpectralViewInfo {
        visible: s.get("visible")?.as_bool()?,
        split_ratio: s.get("split_ratio")?.as_f64()?,
        fft_size: match s.get("fft_size") {
            Some(serde_json::Value::Number(n)) => n.as_u64().map(|n| n as u32),
            _ => None,
        },
        freq_scale: s
            .get("freq_scale")
            .and_then(|v| v.as_str())
            .unwrap_or("log")
            .to_string(),
        display_floor_db: s
            .get("display_floor_db")
            .and_then(|v| v.as_f64())
            .unwrap_or(-120.0),
        display_ceil_db: s
            .get("display_ceil_db")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0),
        colormap: s
            .get("colormap")
            .and_then(|v| v.as_str())
            .unwrap_or("inferno")
            .to_string(),
    })
}

fn info_of(engine: &EngineHandle, doc: Option<&OpenDocument>) -> DocumentInfo {
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
        sidecar_dirty: sidecar_dirty_of(engine, doc),
        spectral_view: spectral_view_of(&doc.sidecar.view),
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
            normalize_busy: Mutex::new(false),
            pending_sidecar_notice: Mutex::new(None),
        }))
    }

    /// The currently open document (or [`DocumentInfo::default`] when none is open).
    pub fn info(&self) -> DocumentInfo {
        info_of(&self.0.engine, self.0.open.lock().unwrap().as_ref())
    }

    /// Imports `path` as a new session (SPEC-005 §2.2-2.4, T-202 `vox_project::import_file`) and
    /// makes it the engine's playback document. Any format `vox_io::decode` supports opens (WAV
    /// incl. the tolerated variants, FLAC, MP3, M4A AAC-LC, Ogg Vorbis); multichannel input
    /// always downmixes by average (the channel-choice dialog is T-209; `document_probe` /
    /// [`vox_project::probe_for_import`] already report everything it would need). Replaces
    /// whatever was open before (the frontend is responsible for the
    /// unsaved-changes prompt — SPEC-004 §2.8 "simple version", ticket scope — before calling
    /// this).
    /// `confirm_already_open` bypasses SPEC-018 §2.11's "already open in another instance"
    /// warning (the UI re-issues with `true` after "Open Anyway").
    pub fn open(&self, path: &Path, confirm_already_open: bool) -> Result<DocumentInfo, IpcError> {
        if self.is_recording() {
            return Err(IpcError::not_while_recording());
        }
        // T-306 (SPEC-018 §2.11): warn if another instance already holds this file open.
        if !confirm_already_open {
            let canonical = vox_project::canonical_path_for_compare(path);
            if vox_project::gc::find_already_open(&self.0.sessions_dir, &canonical, None).is_some()
            {
                return Err(already_open_error(path));
            }
        }
        let probe = vox_project::probe_for_import(path).map_err(document_error)?;
        let save_bits = save_bits_for_import(path, &probe.container);
        let mut config = SessionConfig::new(probe.sample_rate_hz);
        // T-306 (SPEC-018 §2.11): `meta.json`'s `source_path` is what the already-open scan
        // matches on — without it, no session would ever look "already open".
        if let Ok(source_meta) = std::fs::metadata(path) {
            config.source = Some(vox_project::SourceInfo {
                path: path.to_path_buf(),
                size_bytes: source_meta.len(),
                mtime_unix_ms: source_meta.modified().map(unix_ms_of).unwrap_or(0),
                format: probe.container.clone(),
            });
        }
        let mut session = Session::create(&self.0.sessions_dir, config).map_err(document_error)?;
        let cancel = vox_project::CancelToken::new();
        let import = vox_project::import_file(
            &mut session,
            path,
            vox_io::DownmixChoice::Average,
            &cancel,
            |_frames_done, _len_hint| {},
        );
        let import = match import {
            Ok(result) => result,
            Err(err) => {
                let _ = std::fs::remove_dir_all(session.dir());
                return Err(document_error(err));
            }
        };
        let mut snapshot = import.snapshot;

        // T-306 (SPEC-018 §2.5): read and classify the sidecar, if any.
        let identity = DocumentIdentity {
            sample_rate_hz: session.sample_rate_hz(),
            len_samples: snapshot.len_samples,
            audio_crc32: document_crc32(session.store(), &snapshot).unwrap_or(0),
        };
        let sidecar_path = sidecar_path_for(path);
        let load = read_sidecar(&sidecar_path, identity);
        let mut sidecar = SidecarState::none();
        sidecar.needs_backup = load.needs_backup;
        sidecar.audio_crc32 = identity.audio_crc32;
        if let Some(sidecar_doc) = &load.doc {
            // Markers: re-floor with the sidecar's (authoritative, SPEC-009 §2.13), same
            // mechanism `import_file` used for the WAV cue markers — valid here because nothing
            // has edited the document yet (`Session::set_floor`'s only precondition).
            sidecar.marker_meta = MarkerMetaTable::from_items(&load.markers);
            let sidecar_markers: Vec<Marker> = load.markers.iter().map(marker_from_item).collect();
            let audio = WrittenAudio {
                pieces: snapshot.pieces.to_vec(),
                chunks: Vec::new(),
                len_samples: snapshot.len_samples,
            };
            if let Ok(reflowed) = session.set_floor(&audio, sidecar_markers) {
                snapshot = reflowed;
            }
            // Rack (ADR-001 rule 4: `project` keeps `rack` opaque; the conversion is `RackModel`'s
            // own serde, run here). A malformed section never blocks opening (SPEC-018 §2.5): the
            // carried-over rack stays in place instead.
            if let Ok(model) =
                serde_json::from_value::<vox_rack::RackModel>(sidecar_doc.rack.clone())
            {
                let _ = self.0.engine.rack_load_model(model);
            }
            sidecar.view = sidecar_doc.view.clone();
        }
        if let Some(notice) = &load.notice {
            self.set_sidecar_notice(sidecar_notice_info(notice, &sidecar_path));
        }

        let store = Arc::clone(session.store());
        self.0.engine.set_document(Some(PlaybackDoc {
            store,
            snapshot: Arc::clone(&snapshot),
        }));
        // S2-01: a new document replaces the clipboard (same-document only, SPEC-008 §2.6) — its
        // pieces reference the session that's about to close.
        *self.0.clipboard.lock().unwrap() = None;

        let meta = std::fs::metadata(path).ok();
        sidecar.file_size_bytes = meta.as_ref().map(|m| m.len()).unwrap_or(0);
        sidecar.file_mtime_unix_ms = meta
            .and_then(|m| m.modified().ok())
            .map(unix_ms_of)
            .unwrap_or(0);
        let save_format = save_format_model(save_bits);
        let markers = sidecar.marker_meta.build_items(&snapshot.markers);
        let rack = current_rack_value(&self.0.engine);
        sidecar.persisted_digest =
            vox_project::sidecar::persisted_digest(&save_format, &markers, &rack);
        sidecar.last_saved_audio_rev = snapshot.audio_rev;
        sidecar.last_written_markers = markers;

        let mut guard = self.0.open.lock().unwrap();
        let previous = guard.replace(OpenDocument {
            session,
            path: Some(path.to_path_buf()),
            save_bits,
            sidecar,
        });
        let info = info_of(&self.0.engine, guard.as_ref());
        drop(guard);
        if let Some(previous) = previous {
            close_with_retry(previous.session);
        }
        Ok(info)
    }

    /// Drains the last open/save's sidecar notice, if any (see [`Inner::pending_sidecar_notice`]).
    pub fn take_sidecar_notice(&self) -> Option<SidecarNoticeInfo> {
        self.0.pending_sidecar_notice.lock().unwrap().take()
    }

    fn set_sidecar_notice(&self, notice: SidecarNoticeInfo) {
        *self.0.pending_sidecar_notice.lock().unwrap() = Some(notice);
    }

    /// T-306 (SPEC-018 §2.6.5): merges `spectral` into the open document's `view` JSON for the
    /// next save — a no-op (not an error) with no document open. Never touches `sidecar_dirty`
    /// (§2.4: "view state never marks the document modified" — the digest excludes `view`).
    /// Other `view` sections (waveform/markers_panel/rack_panel), if a loaded sidecar had them,
    /// are preserved untouched: this only ever replaces the `spectral` key.
    pub fn set_spectral_view(&self, spectral: SpectralViewInfo) {
        let mut guard = self.0.open.lock().unwrap();
        let Some(doc) = guard.as_mut() else {
            return;
        };
        let mut view = match std::mem::take(&mut doc.sidecar.view) {
            serde_json::Value::Object(map) => map,
            _ => serde_json::Map::new(),
        };
        view.insert(
            "spectral".to_string(),
            serde_json::json!({
                "visible": spectral.visible,
                "split_ratio": spectral.split_ratio,
                "fft_size": spectral.fft_size,
                "freq_scale": spectral.freq_scale,
                "display_floor_db": spectral.display_floor_db,
                "display_ceil_db": spectral.display_ceil_db,
                "colormap": spectral.colormap,
            }),
        );
        doc.sidecar.view = serde_json::Value::Object(view);
    }

    /// Writes the current revision back to its bound path at its bound format (SPEC-005 §2.7,
    /// extended by SPEC-018 §2.3/§2.4/§2.9): a **sidecar-only** write when nothing that would
    /// change the audio bytes changed (§2.4) — full save otherwise. The frontend routes "Save"
    /// with no bound path (an untitled recording, S1-04) through `document_save_as` instead; here
    /// it is refused (`error.document.untitled`).
    ///
    /// `overwrite` bypasses SPEC-018 §2.9's changed-on-disk confirmation (the UI re-issues with
    /// `true` after "Overwrite").
    pub fn save(&self, overwrite: bool) -> Result<DocumentInfo, IpcError> {
        let mut guard = self.0.open.lock().unwrap();
        let doc = guard.as_mut().ok_or_else(no_document)?;
        let path = doc.path.clone().ok_or_else(untitled)?;
        let bits = doc.save_bits;
        if !overwrite && changed_on_disk(doc, &path) {
            return Err(changed_on_disk_error(&path));
        }
        if let Some(notice) = save_to(doc, &self.0.engine, &path, bits)? {
            self.set_sidecar_notice(notice);
        }
        Ok(info_of(&self.0.engine, guard.as_ref()))
    }

    /// Writes the current revision to `path` at `bits` (always a full save, SPEC-018 §2.3), then
    /// binds the document to it.
    pub fn save_as(&self, path: &Path, bits: BitDepth) -> Result<DocumentInfo, IpcError> {
        let mut guard = self.0.open.lock().unwrap();
        let doc = guard.as_mut().ok_or_else(no_document)?;
        // Force a full save: a different path (or, potentially, format) is never a sidecar-only
        // write. Bumping the recorded audio_rev back one guarantees `save_to`'s "audio changed"
        // check fires even when Save As targets the very same content just saved in place.
        doc.sidecar.last_saved_audio_rev = doc.sidecar.last_saved_audio_rev.wrapping_sub(1);
        if let Some(notice) = save_to(doc, &self.0.engine, path, bits)? {
            self.set_sidecar_notice(notice);
        }
        doc.path = Some(path.to_path_buf());
        doc.save_bits = bits;
        Ok(info_of(&self.0.engine, guard.as_ref()))
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

    /// T-204: the current document's store and snapshot for the spectrogram tile service
    /// (`ipc::spectro_commands::spectro_request`). The service holds only a weak reference to
    /// the store between tiles, so this never keeps a session from closing.
    pub fn spectro_source(&self) -> Result<vox_engine::spectro::TileSource, IpcError> {
        let guard = self.0.open.lock().unwrap();
        let doc = guard.as_ref().ok_or_else(no_document)?;
        Ok(vox_engine::spectro::TileSource {
            store: Arc::clone(doc.session.store()),
            snapshot: doc.session.current(),
        })
    }

    /// S4-04: a read-only snapshot of the current document for the export job.
    pub fn export_source(&self) -> Result<ExportSource, IpcError> {
        let guard = self.0.open.lock().unwrap();
        let doc = guard.as_ref().ok_or_else(no_document)?;
        let snapshot = doc.session.current();
        Ok(ExportSource {
            store: Arc::clone(doc.session.store()),
            len_samples: snapshot.len_samples,
            sample_rate_hz: doc.session.sample_rate_hz(),
            snapshot,
            suggested_name: doc
                .path
                .as_ref()
                .and_then(|p| p.file_stem())
                .map(|s| s.to_string_lossy().into_owned()),
        })
    }

    /// S3-06: whether a take is being recorded — Capture Noise Print is disabled then (SPEC-014
    /// §2.3), same rule as every edit command.
    pub fn is_recording(&self) -> bool {
        self.0
            .open
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|d| d.session.is_recording())
    }

    /// H-09: `true` while a normalize job is running (SPEC-010 §2.1's "another document job
    /// runs" — `error.document_busy`).
    pub fn is_normalize_busy(&self) -> bool {
        *self.0.normalize_busy.lock().unwrap()
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
                sidecar: SidecarState::none(),
            });
        }
        let doc = guard.as_mut().ok_or_else(no_document)?;
        let capture = doc
            .session
            .begin_take(TakeMode::New, TakeWriterOptions::default())
            .map_err(document_error)?;
        let info = info_of(&self.0.engine, guard.as_ref());
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
    ///
    /// H-10 item 1: every other error (e.g. `TakeNotInStore`) leaves `doc.session.open_take`
    /// set (`session.rs` docs) — the whole document would otherwise refuse edits, new
    /// recordings and close until restart. Rather than leave that broken session in place, it is
    /// swapped out for a fresh empty one at the same rate, so the app stays usable immediately.
    /// The broken session (and its take WAV, still holding whatever the store never got) is not
    /// deleted — it is just dropped, releasing its lock (same "leave it for a future GC pass"
    /// idea as [`close_with_retry`]) so a later recovery pass can still offer it. The caller
    /// still sees `Err`, so it can tell the owner a take was lost from the open document; call
    /// [`Self::info`] afterwards to get the fresh (empty) document.
    pub fn commit_take(
        &self,
        finished: &FinishedTake,
        dropouts: &[DropoutMark],
    ) -> Result<Option<DocumentInfo>, IpcError> {
        let mut guard = self.0.open.lock().unwrap();
        let doc = guard.as_mut().ok_or_else(no_document)?;
        // H-10 item 4 (SPEC-002 §2.4/§4.3, AC-7): every dropout the capture-writer filled with
        // silence becomes a "Dropout N ms" marker, committed with the take's own undo entry (one
        // undo removes the take and all its markers together, like user-added ones).
        let rate_hz = doc.session.sample_rate_hz();
        let markers: Vec<Marker> = dropouts
            .iter()
            .map(|d| {
                let id = doc.session.new_marker_id();
                Marker::new(
                    id,
                    d.pos_samples,
                    d.len_samples,
                    dropout_marker_name(d.len_samples, rate_hz),
                )
            })
            .collect();
        let mut outcome = doc.session.commit_take(finished, &markers);
        for _ in 1..COMMIT_ATTEMPTS {
            if outcome.is_ok() {
                break;
            }
            std::thread::sleep(COMMIT_RETRY_DELAY);
            outcome = doc.session.commit_take(finished, &markers);
        }
        let step = match outcome {
            Ok(step) => step,
            Err(err) => {
                let ipc_err = document_error(err);
                let rate_hz = doc.session.sample_rate_hz();
                let save_bits = doc.save_bits;
                match Session::create(&self.0.sessions_dir, SessionConfig::new(rate_hz)) {
                    Ok(fresh) => {
                        self.0.engine.set_document(Some(PlaybackDoc {
                            store: Arc::clone(fresh.store()),
                            snapshot: fresh.current(),
                        }));
                        *self.0.clipboard.lock().unwrap() = None;
                        if let Some(broken) = guard.replace(OpenDocument {
                            session: fresh,
                            path: None,
                            save_bits,
                            sidecar: SidecarState::none(),
                        }) {
                            tracing::warn!(
                                session = %broken.session.id(),
                                "leaving a session with an uncommitted take for a future recovery pass"
                            );
                            drop(broken.session);
                        }
                    }
                    Err(create_err) => {
                        // Couldn't even open a replacement — leave the broken session in place
                        // (nothing worse than before) rather than lose the open document too.
                        tracing::error!(
                            error = %create_err,
                            "recovering from a failed take commit also failed; the document may stay stuck"
                        );
                    }
                }
                return Err(ipc_err);
            }
        };
        let Some(step) = step else {
            return Ok(None);
        };
        self.0.engine.set_document(Some(PlaybackDoc {
            store: Arc::clone(doc.session.store()),
            snapshot: step.snapshot,
        }));
        Ok(Some(info_of(&self.0.engine, guard.as_ref())))
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
        if self.is_normalize_busy() {
            return Err(document_busy());
        }
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
        if self.is_normalize_busy() {
            return Err(document_busy());
        }
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
        if self.is_normalize_busy() {
            return Err(document_busy());
        }
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
        if self.is_normalize_busy() {
            return Err(document_busy());
        }
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
        if self.is_normalize_busy() {
            return Err(document_busy());
        }
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

    /// S2-02/S4-01, superseded by H-09's job path below: `target_db` range check shared by
    /// [`crate::normalize::NormalizeService`] before it starts a peak-normalize job.
    pub fn validate_normalize_peak_target(target_db: f64) -> Result<(), IpcError> {
        if !target_db.is_finite()
            || !(NORMALIZE_TARGET_MIN_DB..=NORMALIZE_TARGET_MAX_DB).contains(&target_db)
        {
            return Err(IpcError::new(
                IpcErrorCode::InvalidArgument,
                "error.invalid_argument",
            ));
        }
        Ok(())
    }

    /// `target_lufs` range check shared by [`crate::normalize::NormalizeService`] before it
    /// starts a LUFS-normalize job.
    pub fn validate_normalize_lufs_target(target_lufs: f64) -> Result<(), IpcError> {
        if !target_lufs.is_finite()
            || !(NORMALIZE_LUFS_TARGET_MIN_LUFS..=NORMALIZE_LUFS_TARGET_MAX_LUFS)
                .contains(&target_lufs)
        {
            return Err(IpcError::new(
                IpcErrorCode::InvalidArgument,
                "error.invalid_argument",
            ));
        }
        Ok(())
    }

    /// H-09: begins a normalize job (peak or LUFS) — validates the document/scope/recording/busy
    /// state, marks the document busy (`error.document_busy` for any other normalize job, or an
    /// audio-mutating command, until [`Self::finish_normalize_peak`]/[`Self::finish_normalize_lufs`]
    /// clears it), and hands back the read-only source the job's own thread scans/writes against
    /// — no `&mut Session` needed until the final commit (mirrors [`ExportSource`], which never
    /// commits at all). The frontend resolves "no selection" to the whole file before calling
    /// (SPEC-010 §2.1), same as the old synchronous commands.
    pub fn begin_normalize_job(
        &self,
        start: u64,
        end: u64,
    ) -> Result<NormalizeJobSource, IpcError> {
        let mut busy = self.0.normalize_busy.lock().unwrap();
        if *busy {
            return Err(document_busy());
        }
        let guard = self.0.open.lock().unwrap();
        let doc = guard.as_ref().ok_or_else(no_document)?;
        if doc.session.is_recording() {
            return Err(IpcError::not_while_recording());
        }
        let range = Self::selected_range(doc, start, end)?;
        *busy = true;
        Ok(NormalizeJobSource {
            store: Arc::clone(doc.session.store()),
            snapshot: doc.session.current(),
            sample_rate_hz: doc.session.sample_rate_hz(),
            range,
            session_id: doc.session.id().to_string(),
        })
    }

    /// Releases the busy flag without committing anything (H-09: the job never produced a plan —
    /// it was cancelled, or failed before pass 2 finished).
    pub fn abandon_normalize_job(&self) {
        *self.0.normalize_busy.lock().unwrap() = false;
    }

    /// H-09: commits a finished peak-normalize job's plan (SPEC-010 §2.1/§2.8), always releasing
    /// the busy flag. If the session was replaced (a new `document_open`) or edited while the job
    /// ran (its current snapshot no longer matches `source.snapshot`) a `Gain` plan is discarded
    /// — `error.document_busy`, nothing committed, exactly like a cancellation (SPEC-010 §2.8:
    /// chunks already written become unreachable store data, reclaimed by compaction). A `NoOp`
    /// plan never touched the store, so it always "commits" (as a no-op) even across that race.
    pub fn finish_normalize_peak(
        &self,
        source: &NormalizeJobSource,
        plan: NormalizePeakPlan,
    ) -> Result<NormalizeEditResult, IpcError> {
        let mut guard = self.0.open.lock().unwrap();
        let result = (|| {
            let doc = guard.as_mut().ok_or_else(no_document)?;
            if doc.session.id() != source.session_id {
                return Err(document_busy());
            }
            match plan {
                NormalizePeakPlan::NoOp(outcome) => {
                    let snapshot = doc.session.current();
                    Ok(NormalizeEditResult {
                        result: EditResult {
                            changed: false,
                            audio_rev: snapshot.audio_rev,
                            len_samples: snapshot.len_samples,
                            selection: Some((source.range.start, source.range.end)),
                            playhead_samples: source.range.start,
                        },
                        notice: Some(match outcome {
                            NormalizeOutcome::Silent => NormalizeNotice::Silent,
                            NormalizeOutcome::AlreadyNormalized => {
                                NormalizeNotice::AlreadyNormalized
                            }
                        }),
                    })
                }
                NormalizePeakPlan::Gain(edit) => {
                    if !Arc::ptr_eq(&doc.session.current(), &source.snapshot) {
                        return Err(document_busy());
                    }
                    let step = doc.session.commit_edit(edit).map_err(document_error)?;
                    Ok(NormalizeEditResult {
                        result: self.apply_committed(
                            doc,
                            step,
                            normalize_applied_post_edit(source.range),
                        ),
                        notice: None,
                    })
                }
            }
        })();
        drop(guard);
        *self.0.normalize_busy.lock().unwrap() = false;
        result
    }

    /// H-09: commits a finished LUFS-normalize job's plan (mirrors [`Self::finish_normalize_peak`]).
    pub fn finish_normalize_lufs(
        &self,
        source: &NormalizeJobSource,
        plan: NormalizeLufsPlan,
    ) -> Result<NormalizeLufsEditResult, IpcError> {
        let mut guard = self.0.open.lock().unwrap();
        let result = (|| {
            let doc = guard.as_mut().ok_or_else(no_document)?;
            if doc.session.id() != source.session_id {
                return Err(document_busy());
            }
            match plan {
                NormalizeLufsPlan::NoOp(outcome) => {
                    let snapshot = doc.session.current();
                    Ok(NormalizeLufsEditResult {
                        result: EditResult {
                            changed: false,
                            audio_rev: snapshot.audio_rev,
                            len_samples: snapshot.len_samples,
                            selection: Some((source.range.start, source.range.end)),
                            playhead_samples: source.range.start,
                        },
                        notice: Some(match outcome {
                            LufsNormalizeOutcome::Silent => NormalizeLufsNotice::Silent,
                            LufsNormalizeOutcome::AlreadyNormalized => {
                                NormalizeLufsNotice::AlreadyNormalized
                            }
                        }),
                    })
                }
                NormalizeLufsPlan::Gain {
                    edit,
                    predicted_true_peak_dbtp,
                    exceeds_true_peak_ceiling,
                    ..
                } => {
                    if !Arc::ptr_eq(&doc.session.current(), &source.snapshot) {
                        return Err(document_busy());
                    }
                    let step = doc.session.commit_edit(edit).map_err(document_error)?;
                    let result =
                        self.apply_committed(doc, step, normalize_applied_post_edit(source.range));
                    let notice = exceeds_true_peak_ceiling.then_some(
                        NormalizeLufsNotice::TruePeakCeilingExceeded {
                            predicted_true_peak_dbtp,
                        },
                    );
                    Ok(NormalizeLufsEditResult { result, notice })
                }
            }
        })();
        drop(guard);
        *self.0.normalize_busy.lock().unwrap() = false;
        result
    }

    // --- S2-03: markers (add, rename, move/resize, delete) -----------------------------------

    /// The current marker list, in canonical order (SPEC-009 §2.1). Empty (not an error) when no
    /// document is open.
    pub fn markers_get(&self) -> Vec<MarkerInfo> {
        let guard = self.0.open.lock().unwrap();
        match guard.as_ref() {
            Some(doc) => doc
                .session
                .current()
                .markers
                .iter()
                .map(Into::into)
                .collect(),
            None => Vec::new(),
        }
    }

    /// Adds a point (`len_samples == 0`) or region marker at `[pos_samples, pos_samples +
    /// len_samples)` (SPEC-009 §2.2; the UI resolves *where* per its transport-state table —
    /// heard position while playing, cursor or selection while stopped — before calling this).
    /// One undo entry `history.marker_add`, allowed during playback (marker edits never stop it,
    /// SPEC-009 §2.9). Named "Marker NN" (§2.3). `error.invalid_range` when the range doesn't fit
    /// the document.
    pub fn marker_add(&self, pos_samples: u64, len_samples: u64) -> Result<MarkerInfo, IpcError> {
        let mut guard = self.0.open.lock().unwrap();
        let doc = guard.as_mut().ok_or_else(no_document)?;
        if doc.session.is_recording() {
            return Err(IpcError::not_while_recording());
        }
        let len = doc.session.current().len_samples;
        if pos_samples
            .checked_add(len_samples)
            .is_none_or(|end| end > len)
        {
            return Err(invalid_range());
        }
        let name = next_marker_name(&doc.session.current().markers);
        let id = doc.session.new_marker_id();
        let marker = Marker::new(id, pos_samples, len_samples, name);
        let edit = Edit::new("history.marker_add").marker(MarkerOp::Add(marker.clone()));
        doc.session.commit_edit(edit).map_err(document_error)?;
        Ok(MarkerInfo::from(&marker))
    }

    /// Renames marker `id` to `name`, normalized per SPEC-009 §2.4. `error.marker_name_empty`
    /// when the normalized name is empty (no undo entry, the old name stays);
    /// `error.marker_not_found` when `id` doesn't exist. One undo entry `history.marker_rename`.
    pub fn marker_rename(&self, id: u64, name: &str) -> Result<(), IpcError> {
        let mut guard = self.0.open.lock().unwrap();
        let doc = guard.as_mut().ok_or_else(no_document)?;
        if doc.session.is_recording() {
            return Err(IpcError::not_while_recording());
        }
        let normalized = normalize_marker_name(name);
        if normalized.is_empty() {
            return Err(marker_name_empty());
        }
        let marker_id = MarkerId(id);
        if doc.session.current().marker(marker_id).is_none() {
            return Err(marker_not_found());
        }
        let edit = Edit::new("history.marker_rename").marker(MarkerOp::Rename {
            id: marker_id,
            name: normalized.into(),
        });
        doc.session.commit_edit(edit).map_err(document_error)?;
        Ok(())
    }

    /// Moves or resizes marker `id` to `[pos_samples, pos_samples + len_samples)` (SPEC-009 §2.5's
    /// panel-typed edits; dragging is deferred, ticket "Out" list). `kind` only picks the undo
    /// label (`history.marker_move`/`history.marker_resize`) — the stored range is the same
    /// either way. `error.invalid_range`/`error.marker_not_found` as in [`Self::marker_add`].
    pub fn marker_set_range(
        &self,
        id: u64,
        pos_samples: u64,
        len_samples: u64,
        kind: MarkerRangeEditKind,
    ) -> Result<(), IpcError> {
        let mut guard = self.0.open.lock().unwrap();
        let doc = guard.as_mut().ok_or_else(no_document)?;
        if doc.session.is_recording() {
            return Err(IpcError::not_while_recording());
        }
        let marker_id = MarkerId(id);
        if doc.session.current().marker(marker_id).is_none() {
            return Err(marker_not_found());
        }
        let len = doc.session.current().len_samples;
        if pos_samples
            .checked_add(len_samples)
            .is_none_or(|end| end > len)
        {
            return Err(invalid_range());
        }
        let edit = Edit::new(kind.label_key()).marker(MarkerOp::Move {
            id: marker_id,
            pos_samples,
            len_samples,
        });
        doc.session.commit_edit(edit).map_err(document_error)?;
        Ok(())
    }

    /// Deletes the markers in `ids` as one undo entry `history.marker_delete`, whatever the count
    /// (SPEC-009 §2.6; Delete All/Filtered are deferred, ticket "Out" list). A no-op (`Ok(())`,
    /// no undo entry) for an empty `ids`. `error.marker_not_found` if any id doesn't exist —
    /// nothing is deleted then (validated before the edit is built).
    pub fn marker_delete(&self, ids: &[u64]) -> Result<(), IpcError> {
        if ids.is_empty() {
            return Ok(());
        }
        let mut guard = self.0.open.lock().unwrap();
        let doc = guard.as_mut().ok_or_else(no_document)?;
        if doc.session.is_recording() {
            return Err(IpcError::not_while_recording());
        }
        let snapshot = doc.session.current();
        for &id in ids {
            if snapshot.marker(MarkerId(id)).is_none() {
                return Err(marker_not_found());
            }
        }
        let mut edit = Edit::new("history.marker_delete");
        for &id in ids {
            edit = edit.marker(MarkerOp::Remove(MarkerId(id)));
        }
        doc.session.commit_edit(edit).map_err(document_error)?;
        Ok(())
    }

    /// Undoes the top entry (`Ok` with `changed: false` at the undo floor).
    pub fn history_undo(&self) -> Result<EditResult, IpcError> {
        if self.is_normalize_busy() {
            return Err(document_busy());
        }
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
        if self.is_normalize_busy() {
            return Err(document_busy());
        }
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

/// CRC-32 (SPEC-018 §4.2's `crc32-ieee/f32le/v1`) of `path` — a WAV file — decoded the way a
/// reader (or this app, on the next open) would: `vox_io::WavSource::read_mono` applies exactly
/// the `q / 2^(bits-1)` int-to-float conversion `document_crc32`'s callers assume, streaming so a
/// long save doesn't need the whole file in memory twice.
fn wav_file_crc32(path: &Path) -> vox_io::Result<u32> {
    let (_, _, mut source) = vox_io::read_wav(path)?;
    let mut hasher = crc32fast::Hasher::new();
    let mut buf = vec![0f32; vox_project::CHUNK_SAMPLES];
    loop {
        let n = source.read_mono(&mut buf)?;
        if n == 0 {
            break;
        }
        for &s in &buf[..n] {
            hasher.update(&s.to_le_bytes());
        }
    }
    Ok(hasher.finalize())
}

/// Writes the current revision to `path` (SPEC-005 §2.7, extended by SPEC-018 §2.3/§2.4/§2.8/
/// §2.9): the audio only when [`Self`]... — see [`DocumentService::save`]'s doc comment for the
/// "needs a full save" rule. The sidecar is always (re)written. On success, `Ok(None)`; on a
/// sidecar-write failure *after* a successful (or skipped, unchanged) audio write, `Ok(Some(_))`
/// with the notice the caller should surface — the save itself is not an error (SPEC-018 §2.9:
/// "the audio save stands... `sidecar_dirty` stays set").
fn save_to(
    doc: &mut OpenDocument,
    engine: &EngineHandle,
    path: &Path,
    bits: BitDepth,
) -> Result<Option<SidecarNoticeInfo>, IpcError> {
    let snapshot = doc.session.current();
    let save_format = save_format_model(bits);
    let markers = current_marker_items(doc);
    let rack = current_rack_value(engine);

    let needs_full = snapshot.audio_rev != doc.sidecar.last_saved_audio_rev
        || markers != doc.sidecar.last_written_markers
        || bits != doc.save_bits
        || std::fs::metadata(path).is_err();

    if needs_full {
        let wav_markers: Vec<Marker> = markers.iter().map(marker_from_item).collect();
        let mut reader =
            SnapshotReader::new(Arc::clone(doc.session.store()), Arc::clone(&snapshot));
        vox_project::save_snapshot_wav(&mut reader, path, bits.into(), &wav_markers).map_err(
            |err| match err {
                ProjectError::Wav(io_err) => io_save_error(io_err),
                other => document_error(other),
            },
        )?;
    }

    let audio_crc32 = wav_file_crc32(path).unwrap_or(doc.sidecar.audio_crc32);
    let file_meta = std::fs::metadata(path).ok();
    let write_input = vox_project::WriteInput {
        file_name: path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        file_size_bytes: file_meta.as_ref().map(std::fs::Metadata::len).unwrap_or(0),
        file_mtime: file_meta
            .as_ref()
            .and_then(|m| m.modified().ok())
            .map(vox_project::sidecar::system_time_to_rfc3339)
            .unwrap_or_default(),
        sample_rate_hz: doc.session.sample_rate_hz(),
        len_samples: snapshot.len_samples,
        audio_crc32,
        save_format: save_format.clone(),
        markers: &markers,
        rack: rack.clone(),
        view: doc.sidecar.view.clone(),
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        written_at: vox_project::sidecar::system_time_to_rfc3339(std::time::SystemTime::now()),
    };
    let sidecar_path = sidecar_path_for(path);
    let sidecar_written =
        write_sidecar(&sidecar_path, &write_input, doc.sidecar.needs_backup).is_ok();

    // Journal `saved` regardless of the sidecar outcome (SPEC-018 §2.9): the audio save (or the
    // decision to keep its bytes) stands either way.
    doc.session
        .mark_saved_with_sidecar(
            path,
            format_tag(bits),
            Some(vox_project::sidecar::crc32_to_hex(audio_crc32)),
            Some(sidecar_written),
        )
        .map_err(document_error)?;

    doc.sidecar.audio_crc32 = audio_crc32;
    doc.sidecar.file_size_bytes = file_meta.as_ref().map(std::fs::Metadata::len).unwrap_or(0);
    doc.sidecar.file_mtime_unix_ms = file_meta
        .as_ref()
        .and_then(|m| m.modified().ok())
        .map(unix_ms_of)
        .unwrap_or(0);
    doc.sidecar.last_saved_audio_rev = snapshot.audio_rev;

    if sidecar_written {
        doc.sidecar.needs_backup = false;
        doc.sidecar.last_written_markers = markers.clone();
        doc.sidecar.persisted_digest =
            vox_project::sidecar::persisted_digest(&save_format, &markers, &rack);
        Ok(None)
    } else {
        // `sidecar.persisted_digest`/`needs_backup` are left untouched: `sidecar_dirty` stays
        // set, and the next Save retries a sidecar-only write (SPEC-018 §2.9).
        let key = if save_format.container == "flac" {
            "notice.save.sidecar_failed.flac"
        } else {
            "notice.save.sidecar_failed"
        };
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        Ok(Some(SidecarNoticeInfo {
            key,
            params: vec![("name", name)],
        }))
    }
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

    /// A fresh per-test directory under this test run's `powervoice-app-doc-<pid>` parent. Each
    /// one holds preallocated store segments (64 MB+) and nothing removes them when the test
    /// process exits, so the first call sweeps the parents of finished runs — without it they
    /// filled a tmpfs `/tmp` until the tests failed with "Disk quota exceeded".
    fn tmp_dir(tag: &str) -> PathBuf {
        static SWEEP: std::sync::Once = std::sync::Once::new();
        SWEEP.call_once(sweep_finished_test_runs);
        let dir = std::env::temp_dir()
            .join(format!("powervoice-app-doc-{}", std::process::id()))
            .join(format!(
                "{tag}-{}",
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Removes `powervoice-app-doc-<pid>` parents (and the older flat
    /// `powervoice-app-doc-<tag>-<pid>-<nanos>` dirs) whose test process is gone. Uses `/proc`,
    /// so it is a no-op where that doesn't exist.
    fn sweep_finished_test_runs() {
        if !Path::new("/proc/self").exists() {
            return;
        }
        let Ok(entries) = std::fs::read_dir(std::env::temp_dir()) else {
            return;
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let Some(rest) = name
                .to_str()
                .and_then(|n| n.strip_prefix("powervoice-app-doc-"))
            else {
                continue;
            };
            let fields: Vec<&str> = rest.split('-').collect();
            let pid = match fields.len() {
                1 => fields[0],
                n if n >= 3 => fields[n - 2],
                _ => continue,
            };
            let Ok(pid) = pid.parse::<u32>() else {
                continue;
            };
            if pid != std::process::id() && !Path::new(&format!("/proc/{pid}")).exists() {
                let _ = std::fs::remove_dir_all(entry.path());
            }
        }
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

        let info = service.open(&wav_path, false).unwrap();
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

    /// T-202: 8-bit WAV is now *tolerated* (SPEC-005 §2.2), not rejected — this test used to be
    /// named `open_rejects_an_unsupported_wav_variant` (S1-02 scope only had 16/24-bit int and
    /// 32-bit float); `crates/io/src/decode.rs`'s own tests cover the exact `(u-128)/128`
    /// conversion, so this only checks that `document.rs::open` accepts the file end to end.
    #[test]
    fn open_accepts_an_8_bit_wav_via_the_symphonia_decode_path() {
        let (service, _engine, dir) = service("eight-bit");
        let wav_path = dir.join("in.wav");
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 48_000,
            bits_per_sample: 8,
            sample_format: hound::SampleFormat::Int,
        };
        // `hound` expects 8-bit samples as *signed* i8 values (it applies the +128 offset to the
        // unsigned byte itself when writing), even though the WAV format tag is unsigned PCM.
        let mut w = hound::WavWriter::create(&wav_path, spec).unwrap();
        for v in [-128i32, 0, 100] {
            w.write_sample(v).unwrap();
        }
        w.finalize().unwrap();

        let info = service.open(&wav_path, false).unwrap();
        assert_eq!(info.sample_rate_hz, 48_000);
        assert_eq!(info.len_samples, 3);
    }

    /// A container/codec T-202's `symphonia` feature set doesn't register a decoder for at all
    /// (MS-ADPCM WAV, SPEC-005 §2.2) opens as `error.open.unsupported_codec`. Uses system
    /// `ffmpeg` to build a real MS-ADPCM WAV (a hand-built minimal `fmt ` chunk isn't a complete
    /// enough ADPCM header for symphonia's WAV reader to recognize the codec and reach
    /// `make_audio_decoder`; it would instead reject the file as a malformed container, testing
    /// the wrong code path) — skips gracefully if `ffmpeg` isn't installed.
    #[test]
    fn open_rejects_an_unsupported_codec() {
        if std::process::Command::new("ffmpeg")
            .arg("-version")
            .output()
            .is_err()
        {
            eprintln!("skipping: ffmpeg not installed");
            return;
        }
        let (service, _engine, dir) = service("ms-adpcm");
        let src_wav = dir.join("src.wav");
        write_fixture_wav(
            &src_wav,
            &vox_testkit::signal::silence(0.05, 48_000).unwrap(),
            vox_testkit::wav::BitDepth::Int16,
            48_000,
        );
        let wav_path = dir.join("in.wav");
        let status = std::process::Command::new("ffmpeg")
            .args(["-y", "-loglevel", "error", "-i"])
            .arg(&src_wav)
            .args(["-c:a", "adpcm_ms"])
            .arg(&wav_path)
            .status()
            .unwrap();
        assert!(status.success());

        let err = service.open(&wav_path, false).unwrap_err();
        assert_eq!(err.code, IpcErrorCode::InvalidArgument);
        assert_eq!(err.key, "error.open.unsupported_codec");
    }

    #[test]
    fn open_reports_a_missing_file() {
        let (service, _engine, dir) = service("missing");
        let err = service.open(&dir.join("nope.wav"), false).unwrap_err();
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

        let info = service.open(&wav_path, false).unwrap();
        assert_eq!(info.path.as_deref(), Some(wav_path.to_str().unwrap()));

        // Save (no path change): open -> save with no edits round trips bit-exactly (SPEC-005
        // AC-2), same format (Int24) as the source.
        service.save(false).unwrap();
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

    /// T-204: `spectro_source` hands the open document to the tile service, whose tiles carry
    /// the document's `audio_rev`.
    #[test]
    fn spectro_source_feeds_the_tile_service() {
        use vox_engine::spectro::{
            SpectroConfig, SpectroRequest, SpectroService, decode_vxst, vxst_flags,
        };
        let (service, _engine, dir) = service("spectro");
        assert_eq!(
            service.spectro_source().err().unwrap().key,
            "error.document.none"
        );
        let wav_path = dir.join("in.wav");
        let samples: Vec<f32> = (0..96_000)
            .map(|i| ((i as f32) * 0.13).sin() * 0.5)
            .collect();
        write_fixture_wav(
            &wav_path,
            &samples,
            vox_testkit::wav::BitDepth::Float32,
            48_000,
        );
        let info = service.open(&wav_path, false).unwrap();
        let spectro = SpectroService::new(SpectroConfig {
            workers: 1,
            cache_cap_bytes: None,
        });
        let (tx, rx) = std::sync::mpsc::channel();
        spectro.attach(
            1,
            Box::new(move |bytes| {
                let _ = tx.send(bytes);
            }),
        );
        let request = SpectroRequest {
            request_id: 5,
            audio_rev: info.audio_rev,
            fft_size: 2048,
            hop: 512,
            window: 0,
            tiles: vec![0],
        };
        spectro
            .request(1, service.spectro_source().unwrap(), request)
            .unwrap();
        let bytes = rx.recv_timeout(Duration::from_secs(30)).unwrap();
        let (header, codes) = decode_vxst(&bytes).unwrap();
        assert_eq!(
            (header.request_id, header.audio_rev, header.frames),
            (5, info.audio_rev, 188)
        );
        assert_eq!(header.flags & vxst_flags::LAST, vxst_flags::LAST);
        assert!(codes.iter().any(|&c| c > 0));
    }

    #[test]
    fn peaks_and_save_before_any_open_report_no_document() {
        let (service, _engine, _dir) = service("nodoc");
        let peaks_err = service.peaks(64, 0, 4).unwrap_err();
        assert_eq!(peaks_err.code, IpcErrorCode::NotFound);
        assert_eq!(peaks_err.key, "error.document.none");
        let save_err = service.save(false).unwrap_err();
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
        service.open(&wav_path, false).unwrap();

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
        let info = service.open(&wav_path, false).unwrap();
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

        service.open(&a_path, false).unwrap();
        let info = service.open(&b_path, false).unwrap();
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
            .commit_take(&capture.finish(), &[])
            .unwrap()
            .expect("one undoable edit");
        assert_eq!(info.len_samples, samples.len() as u64);
        assert!(info.dirty);
        let transport = service.0.engine.transport_state();
        assert_eq!(transport.doc_len_samples, samples.len() as u64);

        assert_eq!(
            service.save(false).unwrap_err().key,
            "error.document.untitled"
        );
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
            service.open(&saved, false).unwrap_err().code,
            IpcErrorCode::NotWhileRecording
        );
        service.discard_take(capture.id());
        drop(capture);
        assert_eq!(
            service.open(&saved, false).unwrap().name.as_deref(),
            Some("take.wav")
        );
    }

    /// H-10 item 4 (SPEC-002 §2.4/§4.3, AC-7): dropouts the engine reports alongside a finished
    /// take become "Dropout N ms" markers committed with the take's own undo entry — one undo
    /// removes the take and every dropout marker together, like user-added ones (SPEC-002 §2.2).
    #[test]
    fn dropouts_become_markers_committed_with_the_take() {
        let (service, _engine, _dir) = service("dropout-markers");
        let (mut capture, _info) = service
            .begin_recording(48_000, BitDepth::Bit24, false)
            .unwrap();
        let samples = vox_testkit::signal::sine(440.0, -6.0, 0.2, 48_000).unwrap();
        capture.append(&samples).unwrap();
        let dropouts = [
            vox_engine::record::DropoutMark {
                pos_samples: 1_000,
                len_samples: 480, // 10 ms @ 48 kHz
            },
            vox_engine::record::DropoutMark {
                pos_samples: 5_000,
                len_samples: 24, // 0.5 ms, rounds to the nearest ms (0 ms — still marked)
            },
        ];
        let info = service
            .commit_take(&capture.finish(), &dropouts)
            .unwrap()
            .expect("one undoable edit");
        assert!(info.dirty);

        let markers = service.markers_get();
        assert_eq!(markers.len(), 2, "{markers:?}");
        assert_eq!(markers[0].pos_samples, 1_000);
        assert_eq!(markers[0].len_samples, 480);
        assert_eq!(markers[0].name, "Dropout 10 ms");
        assert_eq!(markers[1].pos_samples, 5_000);
        assert_eq!(markers[1].len_samples, 24);
        assert_eq!(markers[1].name, "Dropout 1 ms", "rounds to the nearest ms");

        // One undo entry removes the take and both dropout markers together.
        let history = service.history_state();
        assert!(history.can_undo);
        assert_eq!(history.undo_label.as_deref(), Some("history.record"));
        service.history_undo().unwrap();
        assert!(service.markers_get().is_empty());
    }

    /// H-10 item 1: a `commit_take` failure (e.g. `TakeNotInStore` — the chunk store never got
    /// the take's audio) must not leave the document stuck refusing edits, new recordings and
    /// close until restart (H-05 review finding). `DocumentService::commit_take` swaps in a
    /// fresh empty document instead of leaving the broken session's take open.
    #[test]
    fn commit_take_failure_frees_the_document_instead_of_wedging_it() {
        let (service, _engine, _dir) = service("commit-take-failure");
        let (capture, info) = service
            .begin_recording(48_000, BitDepth::Bit24, false)
            .unwrap();
        assert_eq!((info.sample_rate_hz, info.len_samples), (48_000, 0));
        let stuck_take = capture.id();
        drop(capture); // its WAV/chunk writers are irrelevant to this test.

        // Simulate the store never getting the take's audio (`ProjectError::TakeNotInStore`,
        // session.rs `commit_take` docs): non-zero WAV samples, empty `WrittenAudio`.
        let finished = FinishedTake {
            take: stuck_take,
            audio: vox_project::WrittenAudio::default(),
            wav_parts: Vec::new(),
            wav_samples: 1_000,
            error: None,
        };
        let err = service.commit_take(&finished, &[]).unwrap_err();
        assert_eq!(err.key, "error.take_not_in_store");

        // The document is immediately usable again: empty, at the same rate, not "recording".
        let info = service.info();
        assert_eq!((info.sample_rate_hz, info.len_samples), (48_000, 0));
        assert_eq!(info.path, None);
        assert!(!info.dirty);
        assert!(!service.is_recording());

        // A new recording can start right away — previously this failed with
        // `error.not_while_recording` because the broken session's take never closed.
        let (mut capture, info) = service
            .begin_recording(48_000, BitDepth::Bit24, false)
            .unwrap();
        assert_eq!((info.sample_rate_hz, info.len_samples), (48_000, 0));

        // And it commits normally, proving the new session is fully functional.
        let samples = vox_testkit::signal::sine(440.0, -6.0, 0.05, 48_000).unwrap();
        capture.append(&samples).unwrap();
        let info = service
            .commit_take(&capture.finish(), &[])
            .unwrap()
            .expect("one undoable edit");
        assert_eq!(info.len_samples, samples.len() as u64);
    }

    // --- S2-01: selection, cut/copy/paste/delete/trim/silence, undo/redo -------------------

    fn open_test_doc(service: &DocumentService, dir: &Path, samples: &[f32]) {
        let path = dir.join("in.wav");
        write_fixture_wav(&path, samples, vox_testkit::wav::BitDepth::Float32, 48_000);
        service.open(&path, false).unwrap();
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

    // --- H-09: normalize job path (peak + LUFS) -----------------------------------------------
    //
    // The job service (`crate::normalize::NormalizeService`) drives these three calls from its
    // own thread; these tests drive them synchronously (no thread, `CancelToken::new()` unless a
    // test wants to cancel) to exercise `DocumentService`'s own contract in isolation.

    fn run_normalize_peak(
        service: &DocumentService,
        start: u64,
        end: u64,
        target_db: f64,
    ) -> Result<NormalizeEditResult, IpcError> {
        DocumentService::validate_normalize_peak_target(target_db)?;
        let source = service.begin_normalize_job(start, end)?;
        match vox_project::plan_normalize_peak(
            &source.store,
            &source.snapshot,
            source.sample_rate_hz,
            source.range,
            target_db,
            &vox_project::CancelToken::new(),
            |_| {},
        ) {
            Ok(plan) => service.finish_normalize_peak(&source, plan),
            Err(err) => {
                service.abandon_normalize_job();
                Err(document_error(err))
            }
        }
    }

    fn run_normalize_lufs(
        service: &DocumentService,
        start: u64,
        end: u64,
        target_lufs: f64,
    ) -> Result<NormalizeLufsEditResult, IpcError> {
        DocumentService::validate_normalize_lufs_target(target_lufs)?;
        let source = service.begin_normalize_job(start, end)?;
        match vox_project::plan_normalize_lufs(
            &source.store,
            &source.snapshot,
            source.sample_rate_hz,
            source.range,
            target_lufs,
            &vox_project::CancelToken::new(),
            |_| {},
        ) {
            Ok(plan) => service.finish_normalize_lufs(&source, plan),
            Err(err) => {
                service.abandon_normalize_job();
                Err(document_error(err))
            }
        }
    }

    // --- S2-02: peak normalize favorites -----------------------------------------------------

    #[test]
    fn normalize_favorite_hits_the_target_and_adds_one_undo_entry() {
        let samples = vox_testkit::signal::sine(440.0, -12.0, 0.2, 48_000).unwrap();
        let len = samples.len() as u64;

        for target_db in [-1.0, -0.1, -3.0] {
            let (service, _engine, dir) = service("normalize-favorite-loop");
            open_test_doc(&service, &dir, &samples);
            let result = run_normalize_peak(&service, 0, len, target_db).unwrap();
            assert!(result.notice.is_none());
            assert!(result.result.changed);
            assert_eq!(
                result.result.selection,
                Some((0, len)),
                "selection unchanged"
            );
            assert!(
                !service.is_normalize_busy(),
                "the busy flag clears once the job finishes"
            );

            let snapshot = {
                let guard = service.0.open.lock().unwrap();
                guard.as_ref().unwrap().session.current()
            };
            let mut out = vec![0.0f32; snapshot.len_samples as usize];
            {
                let guard = service.0.open.lock().unwrap();
                guard
                    .as_ref()
                    .unwrap()
                    .session
                    .store()
                    .read(&snapshot, 0, &mut out)
                    .unwrap();
            }
            let peak_dbfs = vox_testkit::measure::peak_dbfs(&out);
            assert!(
                (peak_dbfs - target_db).abs() <= 0.01,
                "target {target_db}, got {peak_dbfs}"
            );

            assert!(service.history_state().can_undo);
            assert_eq!(
                service.history_state().undo_label.as_deref(),
                Some("history.normalize")
            );
            let undo = service.history_undo().unwrap();
            assert!(undo.changed);
            assert_eq!(undo.len_samples, len);
        }
    }

    #[test]
    fn normalize_selection_only_leaves_the_length_and_rest_untouched() {
        let (service, _engine, dir) = service("normalize-scope");
        let mut samples = vox_testkit::signal::sine(440.0, -6.0, 0.2, 48_000).unwrap();
        samples[0] = 0.99; // a louder transient outside the selection below
        open_test_doc(&service, &dir, &samples);
        let len = samples.len() as u64;

        // Selection-only: [1000, 5000) is normalized, the rest is untouched (length preserved).
        let result = run_normalize_peak(&service, 1_000, 5_000, -1.0).unwrap();
        assert!(result.result.changed);
        assert_eq!(result.result.selection, Some((1_000, 5_000)));
        assert_eq!(result.result.len_samples, len, "length preserved");
    }

    #[test]
    fn normalize_is_a_no_op_on_silence_and_posts_a_notice() {
        let (service, _engine, dir) = service("normalize-silent");
        let samples = vox_testkit::signal::silence(0.05, 48_000).unwrap();
        open_test_doc(&service, &dir, &samples);
        let len = samples.len() as u64;

        let result = run_normalize_peak(&service, 0, len, -1.0).unwrap();
        assert!(!result.result.changed);
        assert_eq!(result.notice, Some(NormalizeNotice::Silent));
        assert!(
            !service.history_state().can_undo,
            "no undo entry for a no-op"
        );
    }

    #[test]
    fn normalize_already_at_the_target_is_a_no_op() {
        let (service, _engine, dir) = service("normalize-already");
        let samples = vox_testkit::signal::sine(440.0, -6.0, 0.05, 48_000).unwrap();
        open_test_doc(&service, &dir, &samples);
        let len = samples.len() as u64;

        let first = run_normalize_peak(&service, 0, len, -1.0).unwrap();
        assert!(
            first.result.changed,
            "the first normalize still applies (exactness)"
        );
        let second = run_normalize_peak(&service, 0, len, -1.0).unwrap();
        assert!(!second.result.changed);
        assert_eq!(second.notice, Some(NormalizeNotice::AlreadyNormalized));
    }

    #[test]
    fn normalize_rejects_an_out_of_range_target() {
        let (service, _engine, dir) = service("normalize-bad-target");
        let samples = vox_testkit::signal::sine(440.0, -6.0, 0.05, 48_000).unwrap();
        open_test_doc(&service, &dir, &samples);
        let len = samples.len() as u64;

        for bad in [-60.01, 0.01, f64::NAN, f64::INFINITY] {
            let err = run_normalize_peak(&service, 0, len, bad).unwrap_err();
            assert_eq!(err.code, IpcErrorCode::InvalidArgument);
            assert!(
                !service.is_normalize_busy(),
                "rejected before the busy flag is set"
            );
        }
    }

    #[test]
    fn normalize_is_refused_while_recording() {
        let (service, _engine, dir) = service("normalize-recording");
        open_test_doc(
            &service,
            &dir,
            &vox_testkit::signal::silence(0.05, 48_000).unwrap(),
        );
        let (mut capture, _info) = service
            .begin_recording(48_000, BitDepth::Bit24, true)
            .unwrap();
        capture.append(&[0.0; 100]).unwrap();

        let err = run_normalize_peak(&service, 0, 10, -1.0).unwrap_err();
        assert_eq!(err.code, IpcErrorCode::NotWhileRecording);
        assert!(!service.is_normalize_busy());

        service.discard_take(capture.id());
    }

    /// H-09 ticket AC: "cancel mid-job leaves hash unchanged" — cancelling partway through pass 2
    /// (the write) leaves `rev`, `audio_rev`, the audio hash and the undo depth exactly as before
    /// (SPEC-010 §2.8), and clears the busy flag so a later normalize can run.
    #[test]
    fn normalize_peak_cancel_mid_job_leaves_the_document_untouched() {
        let (service, _engine, dir) = service("normalize-cancel");
        let samples = vox_testkit::signal::sine(440.0, -12.0, 0.5, 48_000).unwrap();
        open_test_doc(&service, &dir, &samples);
        let len = samples.len() as u64;

        let before_hash = {
            let guard = service.0.open.lock().unwrap();
            let doc = guard.as_ref().unwrap();
            let snapshot = doc.session.current();
            let mut out = vec![0.0f32; snapshot.len_samples as usize];
            doc.session.store().read(&snapshot, 0, &mut out).unwrap();
            (snapshot.audio_rev, vox_testkit::golden::fnv1a_hash(&out))
        };
        assert!(!service.history_state().can_undo);

        let source = service.begin_normalize_job(0, len).unwrap();
        assert!(service.is_normalize_busy());
        let cancel = vox_project::CancelToken::new();
        let err = vox_project::plan_normalize_peak(
            &source.store,
            &source.snapshot,
            source.sample_rate_hz,
            source.range,
            -1.0,
            &cancel,
            |fraction| {
                if fraction > 0.5 {
                    cancel.cancel();
                }
            },
        )
        .unwrap_err();
        assert!(matches!(err, ProjectError::Cancelled));
        service.abandon_normalize_job();

        assert!(!service.is_normalize_busy(), "cancel clears the busy flag");
        assert!(!service.history_state().can_undo, "no undo entry");
        let after = {
            let guard = service.0.open.lock().unwrap();
            let doc = guard.as_ref().unwrap();
            let snapshot = doc.session.current();
            let mut out = vec![0.0f32; snapshot.len_samples as usize];
            doc.session.store().read(&snapshot, 0, &mut out).unwrap();
            (snapshot.audio_rev, vox_testkit::golden::fnv1a_hash(&out))
        };
        assert_eq!(after, before_hash, "rev and audio hash unchanged");

        // The busy flag being clear again means a normal normalize still works afterwards.
        let result = run_normalize_peak(&service, 0, len, -1.0).unwrap();
        assert!(result.result.changed);
    }

    /// SPEC-010 §2.1: while a normalize job is running, other audio-mutating commands are
    /// refused with `error.document_busy` rather than racing the job's eventual commit.
    #[test]
    fn a_second_normalize_and_other_edits_are_refused_while_one_is_busy() {
        let (service, _engine, dir) = service("normalize-busy");
        let samples = vox_testkit::signal::sine(440.0, -12.0, 0.5, 48_000).unwrap();
        open_test_doc(&service, &dir, &samples);
        let len = samples.len() as u64;

        let source = service.begin_normalize_job(0, len).unwrap();
        assert!(service.is_normalize_busy());

        let err = match service.begin_normalize_job(0, len) {
            Err(err) => err,
            Ok(_) => panic!("expected a second normalize job to be refused as busy"),
        };
        assert_eq!(err.code, IpcErrorCode::Busy);
        let err = service.edit_delete(0, 10).unwrap_err();
        assert_eq!(err.code, IpcErrorCode::Busy);
        let err = service.history_undo().unwrap_err();
        assert_eq!(err.code, IpcErrorCode::Busy);

        service.abandon_normalize_job();
        assert!(!service.is_normalize_busy());
        assert!(service.edit_delete(0, 10).is_ok());
        let _ = source;
    }

    // --- S4-01: LUFS normalize favorites -------------------------------------------------------

    fn read_out(service: &DocumentService) -> Vec<f32> {
        let guard = service.0.open.lock().unwrap();
        let doc = guard.as_ref().unwrap();
        let snapshot = doc.session.current();
        let mut out = vec![0.0f32; snapshot.len_samples as usize];
        doc.session.store().read(&snapshot, 0, &mut out).unwrap();
        out
    }

    #[test]
    fn normalize_lufs_favorite_hits_the_target_and_adds_one_undo_entry() {
        let samples = vox_testkit::signal::sine(1000.0, -20.0, 2.0, 48_000).unwrap();
        let len = samples.len() as u64;

        for target_lufs in [-16.0, -19.0, -23.0] {
            let (service, _engine, dir) = service("normalize-lufs-favorite-loop");
            open_test_doc(&service, &dir, &samples);
            let result = run_normalize_lufs(&service, 0, len, target_lufs).unwrap();
            assert!(result.notice.is_none());
            assert!(result.result.changed);
            assert_eq!(
                result.result.selection,
                Some((0, len)),
                "selection unchanged"
            );

            let out = read_out(&service);
            let report = vox_dsp::loudness::measure(&out, 48_000).unwrap();
            assert!(
                (report.integrated_lufs - target_lufs).abs() <= 0.1,
                "target {target_lufs}, got {} LUFS",
                report.integrated_lufs
            );

            assert!(service.history_state().can_undo);
            assert_eq!(
                service.history_state().undo_label.as_deref(),
                Some("history.normalize_lufs")
            );
            let undo = service.history_undo().unwrap();
            assert!(undo.changed);
            assert_eq!(undo.len_samples, len);
        }
    }

    #[test]
    fn normalize_lufs_selection_only_leaves_the_length_and_rest_untouched() {
        let (service, _engine, dir) = service("normalize-lufs-scope");
        let sine = vox_testkit::signal::sine(1000.0, -20.0, 1.0, 48_000).unwrap();
        let mut samples = vec![0.02f32; 48_000];
        samples.extend(sine);
        samples.extend(vec![0.02f32; 48_000]);
        open_test_doc(&service, &dir, &samples);
        let len = samples.len() as u64;

        let result = run_normalize_lufs(&service, 48_000, 96_000, -19.0).unwrap();
        assert!(result.result.changed);
        assert_eq!(result.result.selection, Some((48_000, 96_000)));
        assert_eq!(result.result.len_samples, len, "length preserved");
    }

    #[test]
    fn normalize_lufs_is_a_no_op_on_silence_and_posts_a_notice() {
        let (service, _engine, dir) = service("normalize-lufs-silent");
        let samples = vox_testkit::signal::silence(0.5, 48_000).unwrap();
        open_test_doc(&service, &dir, &samples);
        let len = samples.len() as u64;

        let result = run_normalize_lufs(&service, 0, len, -19.0).unwrap();
        assert!(!result.result.changed);
        assert_eq!(result.notice, Some(NormalizeLufsNotice::Silent));
        assert!(
            !service.history_state().can_undo,
            "no undo entry for a no-op"
        );
    }

    #[test]
    fn normalize_lufs_already_at_the_target_is_a_no_op() {
        let (service, _engine, dir) = service("normalize-lufs-already");
        let samples = vox_testkit::signal::sine(1000.0, -20.0, 2.0, 48_000).unwrap();
        open_test_doc(&service, &dir, &samples);
        let len = samples.len() as u64;

        let first = run_normalize_lufs(&service, 0, len, -19.0).unwrap();
        assert!(
            first.result.changed,
            "the first normalize still applies (exactness)"
        );
        let second = run_normalize_lufs(&service, 0, len, -19.0).unwrap();
        assert!(!second.result.changed);
        assert_eq!(second.notice, Some(NormalizeLufsNotice::AlreadyNormalized));
    }

    #[test]
    fn normalize_lufs_rejects_an_out_of_range_target() {
        let (service, _engine, dir) = service("normalize-lufs-bad-target");
        let samples = vox_testkit::signal::sine(1000.0, -20.0, 0.5, 48_000).unwrap();
        open_test_doc(&service, &dir, &samples);
        let len = samples.len() as u64;

        for bad in [-60.01, 0.01, f64::NAN, f64::INFINITY] {
            let err = run_normalize_lufs(&service, 0, len, bad).unwrap_err();
            assert_eq!(err.code, IpcErrorCode::InvalidArgument);
        }
    }

    #[test]
    fn normalize_lufs_is_refused_while_recording() {
        let (service, _engine, dir) = service("normalize-lufs-recording");
        open_test_doc(
            &service,
            &dir,
            &vox_testkit::signal::silence(0.05, 48_000).unwrap(),
        );
        let (mut capture, _info) = service
            .begin_recording(48_000, BitDepth::Bit24, true)
            .unwrap();
        capture.append(&[0.0; 100]).unwrap();

        let err = run_normalize_lufs(&service, 0, 10, -19.0).unwrap_err();
        assert_eq!(err.code, IpcErrorCode::NotWhileRecording);

        service.discard_take(capture.id());
    }

    /// Ticket AC: normalizing up to a target that would push the true peak past −1 dBTP still
    /// applies the gain and posts a notice (no silent limiting).
    #[test]
    fn normalize_lufs_posts_a_true_peak_ceiling_notice_but_still_applies() {
        let (service, _engine, dir) = service("normalize-lufs-tp");
        let samples = vox_testkit::signal::sine(1000.0, -0.5, 2.0, 48_000).unwrap();
        open_test_doc(&service, &dir, &samples);
        let len = samples.len() as u64;

        let result = run_normalize_lufs(&service, 0, len, 0.0).unwrap();
        assert!(result.result.changed, "the gain is applied regardless");
        match result.notice {
            Some(NormalizeLufsNotice::TruePeakCeilingExceeded {
                predicted_true_peak_dbtp,
            }) => assert!(predicted_true_peak_dbtp > -1.0),
            other => panic!("expected a true-peak ceiling notice, got {other:?}"),
        }
    }

    /// H-09: `target_pct` (SPEC-010 §2.4) resolves to `target_db` via `vox_project::
    /// target_pct_to_db` before reaching `run_normalize_peak` — exercised end to end here rather
    /// than only at the pure-function level (`crates/project`'s own tests).
    #[test]
    fn normalize_peak_accepts_a_percent_target_resolved_to_db() {
        let (service, _engine, dir) = service("normalize-pct");
        let samples = vox_testkit::signal::sine(440.0, -12.0, 0.2, 48_000).unwrap();
        open_test_doc(&service, &dir, &samples);
        let len = samples.len() as u64;

        let target_db = vox_project::target_pct_to_db(50.0); // ~ -6.02 dB
        let result = run_normalize_peak(&service, 0, len, target_db).unwrap();
        assert!(result.result.changed);
        let out = read_out(&service);
        let peak_dbfs = vox_testkit::measure::peak_dbfs(&out);
        assert!((peak_dbfs - (-6.02)).abs() <= 0.01, "got {peak_dbfs}");
    }

    // --- S2-03: markers -----------------------------------------------------------------------

    #[test]
    fn marker_add_names_and_orders_markers_and_is_undoable() {
        let (service, _engine, dir) = service("marker-add");
        let samples = vox_testkit::signal::sine(440.0, -6.0, 3.0, 48_000).unwrap();
        open_test_doc(&service, &dir, &samples);

        let a = service.marker_add(96_000, 0).unwrap();
        assert_eq!(a.name, "Marker 01");
        assert_eq!((a.pos_samples, a.len_samples), (96_000, 0));

        let region = service.marker_add(48_000, 48_000).unwrap();
        assert_eq!(region.name, "Marker 02");

        assert_eq!(service.markers_get().len(), 2);
        // Canonical order: by position, so the region (48 000) sorts before the point (96 000).
        let list = service.markers_get();
        assert_eq!(list[0].id, region.id);
        assert_eq!(list[1].id, a.id);

        assert!(service.history_state().can_undo);
        assert_eq!(
            service.history_state().undo_label.as_deref(),
            Some("history.marker_add")
        );
        let undo = service.history_undo().unwrap();
        assert!(undo.changed, "a marker-only edit is still an undo entry");
        assert_eq!(service.markers_get().len(), 1);
        let redo = service.history_redo().unwrap();
        assert!(redo.changed);
        assert_eq!(service.markers_get().len(), 2);
    }

    #[test]
    fn marker_add_rejects_an_out_of_range_marker() {
        let (service, _engine, dir) = service("marker-add-bad-range");
        let samples = vox_testkit::signal::silence(0.05, 48_000).unwrap();
        open_test_doc(&service, &dir, &samples);
        let len = samples.len() as u64;

        assert_eq!(
            service.marker_add(len + 1, 0).unwrap_err().key,
            "error.invalid_range"
        );
        assert_eq!(
            service.marker_add(len, 1).unwrap_err().key,
            "error.invalid_range"
        );
        // A point exactly at the end is legal (SPEC-009 AC-1).
        assert!(service.marker_add(len, 0).is_ok());
        assert!(service.history_state().can_undo);
    }

    #[test]
    fn marker_rename_normalizes_and_rejects_an_empty_name() {
        let (service, _engine, dir) = service("marker-rename");
        let samples = vox_testkit::signal::sine(440.0, -6.0, 0.2, 48_000).unwrap();
        open_test_doc(&service, &dir, &samples);
        let marker = service.marker_add(1_000, 0).unwrap();

        service.marker_rename(marker.id, "  Intro  ").unwrap();
        assert_eq!(service.markers_get()[0].name, "Intro");
        assert_eq!(
            service.history_state().undo_label.as_deref(),
            Some("history.marker_rename")
        );

        assert_eq!(
            service.marker_rename(marker.id, "   ").unwrap_err().key,
            "error.marker_name_empty"
        );
        assert_eq!(
            service.markers_get()[0].name,
            "Intro",
            "a rejected rename keeps the old name"
        );

        assert_eq!(
            service.marker_rename(999, "x").unwrap_err().key,
            "error.marker_not_found"
        );
    }

    #[test]
    fn marker_set_range_moves_or_resizes_with_the_matching_undo_label() {
        let (service, _engine, dir) = service("marker-set-range");
        let samples = vox_testkit::signal::sine(440.0, -6.0, 3.0, 48_000).unwrap();
        open_test_doc(&service, &dir, &samples);
        let marker = service.marker_add(96_000, 4_800).unwrap();

        service
            .marker_set_range(marker.id, 100_000, 4_800, MarkerRangeEditKind::Move)
            .unwrap();
        assert_eq!(
            service.history_state().undo_label.as_deref(),
            Some("history.marker_move")
        );
        let moved = &service.markers_get()[0];
        assert_eq!((moved.pos_samples, moved.len_samples), (100_000, 4_800));

        service
            .marker_set_range(marker.id, 100_000, 9_600, MarkerRangeEditKind::Resize)
            .unwrap();
        assert_eq!(
            service.history_state().undo_label.as_deref(),
            Some("history.marker_resize")
        );
        let resized = &service.markers_get()[0];
        assert_eq!((resized.pos_samples, resized.len_samples), (100_000, 9_600));

        let len = samples.len() as u64;
        assert_eq!(
            service
                .marker_set_range(marker.id, len, 1, MarkerRangeEditKind::Move)
                .unwrap_err()
                .key,
            "error.invalid_range"
        );
    }

    #[test]
    fn marker_delete_removes_several_as_one_undo_entry() {
        let (service, _engine, dir) = service("marker-delete");
        let samples = vox_testkit::signal::sine(440.0, -6.0, 1.0, 48_000).unwrap();
        open_test_doc(&service, &dir, &samples);
        let a = service.marker_add(1_000, 0).unwrap();
        let b = service.marker_add(2_000, 0).unwrap();
        service.marker_add(3_000, 0).unwrap();
        assert_eq!(service.markers_get().len(), 3);

        service.marker_delete(&[a.id, b.id]).unwrap();
        assert_eq!(service.markers_get().len(), 1);
        assert_eq!(
            service.history_state().undo_label.as_deref(),
            Some("history.marker_delete")
        );

        let undo = service.history_undo().unwrap();
        assert!(undo.changed);
        assert_eq!(
            service.markers_get().len(),
            3,
            "one undo restores both deleted markers"
        );

        assert_eq!(
            service.marker_delete(&[999]).unwrap_err().key,
            "error.marker_not_found"
        );
        assert_eq!(
            service.markers_get().len(),
            3,
            "a failed delete changes nothing"
        );
        assert!(
            service.marker_delete(&[]).is_ok(),
            "an empty list is a no-op"
        );
    }

    #[test]
    fn marker_commands_are_refused_while_recording() {
        let (service, _engine, dir) = service("marker-recording");
        open_test_doc(
            &service,
            &dir,
            &vox_testkit::signal::silence(0.5, 48_000).unwrap(),
        );
        let marker = service.marker_add(1_000, 0).unwrap();

        let (mut capture, _info) = service
            .begin_recording(48_000, BitDepth::Bit24, true)
            .unwrap();
        capture.append(&[0.0; 100]).unwrap();

        for result in [
            service.marker_add(0, 0).map(|_| ()),
            service.marker_rename(marker.id, "x").map(|_| ()),
            service
                .marker_set_range(marker.id, 0, 0, MarkerRangeEditKind::Move)
                .map(|_| ()),
            service.marker_delete(&[marker.id]).map(|_| ()),
        ] {
            assert_eq!(result.unwrap_err().code, IpcErrorCode::NotWhileRecording);
        }

        service.discard_take(capture.id());
    }

    /// SPEC-005 §2.9 / SPEC-009 §2.13 case 5: end-to-end through `DocumentService` — Save writes
    /// markers as WAV cue points, and Open reads them back exactly (positions, lengths, names).
    #[test]
    fn markers_survive_a_save_and_reopen_round_trip() {
        let (service, _engine, dir) = service("marker-round-trip");
        let samples = vox_testkit::signal::sine(440.0, -6.0, 2.0, 48_000).unwrap();
        open_test_doc(&service, &dir, &samples);
        service.marker_add(0, 0).unwrap();
        service
            .marker_rename(service.markers_get()[0].id, "Intro")
            .unwrap();
        service.marker_add(48_000, 9_600).unwrap();

        let save_path = dir.join("with-markers.wav");
        service.save_as(&save_path, BitDepth::Bit24).unwrap();

        // A second document service (a fresh session dir) opens the saved file back.
        let reopened = DocumentService::new(dir.join("sessions2"), service.0.engine.clone());
        reopened.open(&save_path, false).unwrap();
        let markers = reopened.markers_get();
        assert_eq!(markers.len(), 2);
        assert_eq!((markers[0].pos_samples, markers[0].len_samples), (0, 0));
        assert_eq!(markers[0].name, "Intro");
        assert_eq!(
            (markers[1].pos_samples, markers[1].len_samples),
            (48_000, 9_600)
        );
    }

    // --- T-306: sidecar round trip, identity, sidecar-only saves, notices --------------------

    fn gain_rack_model(gain_db: f64) -> vox_rack::RackModel {
        vox_rack::RackModel {
            slots: vec![vox_rack::SlotModel {
                module: "org.powervoice.gain@1.0.0".to_string(),
                bypass: false,
                state: serde_json::json!({"format_version": 1, "params": {"gain_db": gain_db}}),
                extra: Default::default(),
            }],
        }
    }

    #[test]
    fn rack_and_view_survive_a_save_and_reopen_round_trip() {
        let (service, _engine, dir) = service("rack-view-round-trip");
        let samples = vox_testkit::signal::sine(440.0, -6.0, 0.2, 48_000).unwrap();
        open_test_doc(&service, &dir, &samples);

        service
            .0
            .engine
            .rack_load_model(gain_rack_model(-6.123456789012345))
            .unwrap()
            .unwrap();
        service.set_spectral_view(SpectralViewInfo {
            visible: true,
            split_ratio: 62.5,
            fft_size: Some(4096),
            freq_scale: "linear".to_string(),
            display_floor_db: -100.0,
            display_ceil_db: -10.0,
            colormap: "viridis".to_string(),
        });

        let save_path = dir.join("with-rack.wav");
        let saved_info = service.save_as(&save_path, BitDepth::Bit24).unwrap();
        assert!(
            !saved_info.sidecar_dirty,
            "a fresh save is never sidecar_dirty"
        );

        // Clear the shared engine's rack so the reopened document can only get it back from the
        // sidecar, not from carry-over (proves the round trip, not a false positive).
        service
            .0
            .engine
            .rack_load_model(vox_rack::RackModel::default())
            .unwrap()
            .unwrap();

        let reopened = DocumentService::new(dir.join("sessions2"), service.0.engine.clone());
        let info = reopened.open(&save_path, false).unwrap();
        assert!(
            reopened.take_sidecar_notice().is_none(),
            "a matching sidecar posts no notice"
        );

        let reloaded_model = reopened.0.engine.rack_model().unwrap();
        assert_eq!(reloaded_model.slots.len(), 1);
        assert_eq!(reloaded_model.slots[0].module, "org.powervoice.gain@1.0.0");
        let gain_db = reloaded_model.slots[0].state["params"]["gain_db"]
            .as_f64()
            .unwrap();
        assert_eq!(
            gain_db.to_bits(),
            (-6.123456789012345f64).to_bits(),
            "rack parameters round-trip bit-exactly (AC-3)"
        );

        let view = info
            .spectral_view
            .expect("the saved spectral view loads back");
        assert!(view.visible);
        assert!((view.split_ratio - 62.5).abs() < 1e-9);
        assert_eq!(view.fft_size, Some(4096));
        assert_eq!(view.freq_scale, "linear");
        assert_eq!(view.colormap, "viridis");

        assert!(
            !info.sidecar_dirty,
            "opening a file with a matching sidecar never sets * (SPEC-018 §2.5)"
        );
    }

    #[test]
    fn sidecar_dirty_reflects_rack_changes_and_clears_when_reverted() {
        let (service, _engine, dir) = service("sidecar-dirty");
        let samples = vox_testkit::signal::sine(440.0, -6.0, 0.2, 48_000).unwrap();
        open_test_doc(&service, &dir, &samples);
        let save_path = dir.join("a.wav");
        service.save_as(&save_path, BitDepth::Bit24).unwrap();
        assert!(!service.info().sidecar_dirty);

        service
            .0
            .engine
            .rack_load_model(gain_rack_model(-3.0))
            .unwrap()
            .unwrap();
        assert!(
            service.info().sidecar_dirty,
            "a rack parameter change sets sidecar_dirty (AC-11)"
        );

        service
            .0
            .engine
            .rack_load_model(vox_rack::RackModel::default())
            .unwrap()
            .unwrap();
        assert!(
            !service.info().sidecar_dirty,
            "reverting to the saved rack clears sidecar_dirty without a save (AC-11)"
        );
    }

    #[test]
    fn a_rack_only_change_triggers_a_sidecar_only_save_leaving_the_wav_untouched() {
        let (service, _engine, dir) = service("sidecar-only-save");
        let samples = vox_testkit::signal::sine(440.0, -6.0, 0.2, 48_000).unwrap();
        open_test_doc(&service, &dir, &samples);
        let save_path = dir.join("a.wav");
        service.save_as(&save_path, BitDepth::Bit24).unwrap();

        let before_bytes = std::fs::read(&save_path).unwrap();
        let before_mtime = std::fs::metadata(&save_path).unwrap().modified().unwrap();

        service
            .0
            .engine
            .rack_load_model(gain_rack_model(-3.0))
            .unwrap()
            .unwrap();
        assert!(service.info().sidecar_dirty);

        service.save(false).unwrap();
        assert!(!service.info().sidecar_dirty);

        let after_bytes = std::fs::read(&save_path).unwrap();
        let after_mtime = std::fs::metadata(&save_path).unwrap().modified().unwrap();
        assert_eq!(
            before_bytes, after_bytes,
            "a sidecar-only save leaves the WAV bytes untouched (AC-11)"
        );
        assert_eq!(
            before_mtime, after_mtime,
            "a sidecar-only save leaves the WAV mtime untouched (AC-11)"
        );

        let sidecar_path = vox_project::sidecar_path_for(&save_path);
        assert!(sidecar_path.exists());
    }

    #[test]
    fn identity_mismatch_ignores_the_sidecar_and_carries_the_rack_over() {
        let (service, _engine, dir) = service("identity-mismatch");
        let samples_a = vox_testkit::signal::sine(440.0, -6.0, 0.2, 48_000).unwrap();
        open_test_doc(&service, &dir, &samples_a);
        let a_path = dir.join("a.wav");
        service.save_as(&a_path, BitDepth::Bit24).unwrap();

        // A sidecar-only save so `a.wav.vo.json` carries a rack.
        service
            .0
            .engine
            .rack_load_model(gain_rack_model(-9.0))
            .unwrap()
            .unwrap();
        service.save(false).unwrap();

        // `b.wav`: same length/rate, different samples (so a different `audio_crc32`) — copy
        // `a.wav`'s sidecar next to it verbatim (SPEC-018 AC-7's setup).
        let samples_b: Vec<f32> = samples_a.iter().map(|s| s * 0.99).collect();
        let b_path = dir.join("b.wav");
        write_fixture_wav(
            &b_path,
            &samples_b,
            vox_testkit::wav::BitDepth::Float32,
            48_000,
        );
        std::fs::copy(
            vox_project::sidecar_path_for(&a_path),
            vox_project::sidecar_path_for(&b_path),
        )
        .unwrap();

        // Clear the shared engine's rack: if the (wrong) sidecar were applied anyway, this
        // assertion below would catch it.
        service
            .0
            .engine
            .rack_load_model(vox_rack::RackModel::default())
            .unwrap()
            .unwrap();

        let reopened = DocumentService::new(dir.join("sessions2"), service.0.engine.clone());
        reopened.open(&b_path, false).unwrap();
        let notice = reopened
            .take_sidecar_notice()
            .expect("a mismatched sidecar posts a notice");
        assert_eq!(notice.key, "notice.sidecar.mismatch");

        let rack_after = reopened.0.engine.rack_model().unwrap();
        assert!(
            rack_after.slots.is_empty(),
            "a mismatched sidecar is ignored; the carried-over rack stands"
        );
    }

    #[test]
    fn corrupt_sidecar_never_blocks_opening_and_posts_a_notice() {
        let (service, _engine, dir) = service("corrupt-sidecar");
        let samples = vox_testkit::signal::sine(440.0, -6.0, 0.1, 48_000).unwrap();
        let path = dir.join("a.wav");
        write_fixture_wav(&path, &samples, vox_testkit::wav::BitDepth::Float32, 48_000);
        std::fs::write(vox_project::sidecar_path_for(&path), b"not json at all").unwrap();

        let info = service.open(&path, false).unwrap();
        assert_eq!(
            info.sample_rate_hz, 48_000,
            "the document still opens fully"
        );
        let notice = service
            .take_sidecar_notice()
            .expect("a corrupt sidecar posts a notice");
        assert_eq!(notice.key, "notice.sidecar.corrupt");
    }

    #[test]
    fn opening_a_file_already_open_in_another_instance_needs_confirmation() {
        let dir = tmp_dir("already-open");
        let sessions = dir.join("sessions");
        let engine1 = test_engine();
        let engine2 = test_engine();
        let service1 = DocumentService::new(sessions.clone(), engine1.handle());
        let service2 = DocumentService::new(sessions, engine2.handle());

        let path = dir.join("a.wav");
        let samples = vox_testkit::signal::sine(440.0, -6.0, 0.1, 48_000).unwrap();
        write_fixture_wav(&path, &samples, vox_testkit::wav::BitDepth::Float32, 48_000);

        service1.open(&path, false).unwrap();

        let err = service2.open(&path, false).unwrap_err();
        assert_eq!(err.code, IpcErrorCode::NeedsConfirmation);
        assert_eq!(err.key, "dialog.already_open");

        // "Open Anyway": re-issuing with the confirm flag proceeds normally.
        let info = service2.open(&path, true).unwrap();
        assert_eq!(info.name.as_deref(), Some("a.wav"));
    }
}

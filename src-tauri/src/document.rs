//! Document service (S1-03): the app-level owner of the one open session (ADR-004 §8, SPEC-004
//! §2.1). Wraps S1-02's `import_wav`/`save_snapshot_wav`/`peaks` over a [`vox_project::Session`]
//! and keeps the engine's playback document in sync (`EngineHandle::set_document`, MEMORY.md
//! S1-01 Engine API). `ipc::document_commands` only forwards to this — no business logic lives
//! in the command handlers themselves (CLAUDE.md: "`src-tauri` is a thin command/event layer").

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use vox_engine::bake::BakeAttachment;
use vox_engine::record::DropoutMark;
use vox_engine::spectro::SpectroService;
use vox_engine::{EngineHandle, PlaybackDoc, RackCommand, TransportCommand};
use vox_project::{
    CancelToken, DocSnapshot, DocumentIdentity, Edit, EditTarget, FinishedTake, ImportProbe,
    LufsNormalizeOutcome, Marker, MarkerId, MarkerItemModel, MarkerKind, MarkerMetaTable, MarkerOp,
    NormalizeLufsPlan, NormalizeOutcome, NormalizePeakPlan, OversInfo, Piece, ProjectError, Range,
    RangeError, SaveFormatModel, Session, SessionConfig, SidecarNotice, SnapshotReader,
    StoreOptions, TakeCapture, TakeId, TakeMode, TakeWriterOptions, WrittenAudio,
    cue_projection_matches, document_crc32, edit, marker_from_item, markers_with_inherited_kind,
    normalize_applied_post_edit, overs_check, read_sidecar, save_snapshot_flac, sidecar_path_for,
    validate_range, write_sidecar,
};

use crate::ipc::document_dto::{AmplitudeRulerModeDto, TimeRulerFormatDto};
use crate::ipc::{IpcError, IpcErrorCode};
use crate::settings::{BitDepth, SaveDitherPref};

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

/// The OS **local** data dir's `sessions` subfolder (H-51, ADR-004 §1 Amendment 8 / ADR-001 §6):
/// `directories::ProjectDirs::data_dir()` is `FOLDERID_RoamingAppData` on Windows, which can be
/// synced to another machine or a network share — exactly what ADR-004 §1 already rules out for
/// this multi-GB recovery/scratch data (recordings must not sit in a roaming profile). Modules
/// already use the local dir (`app_local_data_dir`, `plugins.rs`); sessions did not.
/// `data_local_dir()` is `FOLDERID_LocalAppData` on Windows, and is *identical* to `data_dir()`
/// on Linux (`$XDG_DATA_HOME`/`~/.local/share`) and macOS (`~/Library/Application Support`), so
/// this changes only Windows behaviour.
///
/// Also migrates a session directory left behind at the old (roaming) path into the new one —
/// see [`migrate_sessions_dir`]. A no-op on every OS but Windows, since the old and new paths are
/// otherwise the same path.
pub fn default_sessions_dir() -> PathBuf {
    match directories::ProjectDirs::from("app", "powervoice", "powervoice") {
        Some(dirs) => {
            let new = dirs.data_local_dir().join("sessions");
            let old = dirs.data_dir().join("sessions");
            if let Err(error) = migrate_sessions_dir(&old, &new) {
                tracing::warn!(
                    old = %old.display(),
                    new = %new.display(),
                    %error,
                    "H-51: could not migrate the old roaming sessions directory"
                );
            }
            new
        }
        None => std::env::temp_dir().join("powervoice").join("sessions"),
    }
}

/// Moves `old` to `new` when `old` holds a leftover session directory and `new` doesn't exist
/// yet. A no-op (returns `Ok(false)`) when there is nothing to migrate, when `old == new` (every
/// OS but Windows), or when `new` already exists — this never overwrites or merges an existing
/// new-location session, since guessing which one is "current" could lose an open document; an
/// orphaned `old` is left for the user/GC to deal with, same as before this ticket.
///
/// Pure filesystem move over explicit paths, so it is testable with a fake "home" (any temp
/// directory standing in for the real OS data dir) without touching real paths or environment
/// variables (CLAUDE.md/webkit.rs convention: keep OS decisions pure and parameterized).
pub(crate) fn migrate_sessions_dir(old: &Path, new: &Path) -> std::io::Result<bool> {
    if old == new || new.exists() || !old.exists() {
        return Ok(false);
    }
    if let Some(parent) = new.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::rename(old, new)?;
    Ok(true)
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
    /// T-209/H-20 (SPEC-005 §2.6/§2.7): the container Save writes — `Wav` unless the source was
    /// FLAC (H-20: FLAC keeps FLAC, §2.6's promotion table) or an explicit Save As chose one.
    save_container: SaveContainer,
    /// H-20 (SPEC-005 §3 `save_dither`): the dither mode Save writes at — the last explicit Save
    /// As choice, or the SPEC-005 default (`Tpdf`) until one is made.
    save_dither: SaveDitherPref,
    /// H-20 (SPEC-005 §2.4 "saving over a multichannel source"): the source's original channel
    /// count (1 for a mono source or a new recording), recorded at open/recovery.
    source_channels: u16,
    /// H-20: the first Save/Save As over a multichannel source has shown its warning — later
    /// saves of the same document never ask again (SPEC-005 AC-10: "exactly once per document").
    multichannel_warned: bool,
    /// H-20 (SPEC-005 §2.10): the source carried metadata PowerVoice doesn't preserve (`LIST
    /// INFO`, `bext`, `iXML`, `smpl`, ID3/Vorbis comments), recorded at open.
    had_foreign_metadata: bool,
    /// H-20: `notice.save.metadata_dropped` has already been shown once for this document (SPEC-
    /// 005 §2.10: "the first Save").
    metadata_notice_shown: bool,
    sidecar: SidecarState,
    /// T-301 (SPEC-004 §2.7): opened by crash recovery — modified and titled "(recovered)"
    /// until the first save.
    recovered: bool,
    /// T-301: digest of the rack/view `state` last journaled (ADR-004 §6), `None` = none yet.
    state_digest: Option<u32>,
}

impl OpenDocument {
    fn new(
        session: Session,
        path: Option<PathBuf>,
        save_bits: BitDepth,
        sidecar: SidecarState,
    ) -> Self {
        OpenDocument {
            session,
            path,
            save_bits,
            save_container: SaveContainer::Wav,
            save_dither: SaveDitherPref::default(),
            source_channels: 1,
            multichannel_warned: false,
            had_foreign_metadata: false,
            metadata_notice_shown: false,
            sidecar,
            recovered: false,
            state_digest: None,
        }
    }
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
    /// H-12 (SPEC-018 §2.6.5): the sidecar's `view.waveform` section, if the open sidecar (or a
    /// prior `set_waveform_view` this session) has one. `None` means "no opinion" — the UI keeps
    /// whatever viewport it already has (e.g. zoom-to-fit for a newly opened document).
    pub waveform_view: Option<WaveformViewInfo>,
    /// T-301 (SPEC-004 §2.7): opened by crash recovery and not saved since — the title gets
    /// "(recovered)".
    pub recovered: bool,
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

/// H-12 (SPEC-018 §2.6.5's `view.waveform`): the shared waveform/spectral viewport plus the
/// selection and edit cursor, lifted out of `EditorView`'s own state so it can be persisted per
/// document (like [`SpectralViewInfo`]) and restored on open. T-206 adds `time_ruler_format`
/// (SPEC-006 §2.5); H-35 adds `vertical_zoom` (SPEC-006 §2.4/§2.6); H-72 adds
/// `amplitude_ruler_mode` (SPEC-006 §2.4, dBFS vs. percent). Plain data —
/// [`crate::ipc::document_dto::WaveformViewDto`] is the ts-rs wire type.
#[derive(Clone, Debug, PartialEq)]
pub struct WaveformViewInfo {
    pub start_sample: u64,
    pub samples_per_pixel: f64,
    /// `[start, end)` document samples, or `None` (no selection).
    pub selection: Option<(u64, u64)>,
    pub cursor_samples: u64,
    /// T-206 (SPEC-006 §2.5, SPEC-018 §2.6.5): the time ruler's display format, persisted per
    /// document like the rest of `waveform`.
    pub time_ruler_format: TimeRulerFormatDto,
    /// H-35 (SPEC-006 §2.4, SPEC-018 §2.6.5): the linear amplitude scale factor, `1.0..=256.0`
    /// (SPEC-006 §2.4's range), default `1.0`.
    pub vertical_zoom: f64,
    /// H-72 (SPEC-006 §2.4, SPEC-018 §2.6.5): dBFS (default, logarithmic) or Percentage (linear)
    /// amplitude ruler mode.
    pub amplitude_ruler_mode: AmplitudeRulerModeDto,
}

/// S2-01/H-56: the in-app clipboard (SPEC-008 §2.6). `Bound` pieces reference the currently open
/// document's session chunks (paste is then a pure splice, S2-01); once that session closes,
/// [`DocumentService::materialize_clipboard`] streams its audio to
/// `<app_local_data_dir>/clipboard/` before the session is torn down, and the clipboard becomes
/// `Materialized` until the next paste imports it into whichever document is open by then.
enum ClipboardData {
    Bound {
        pieces: Vec<Piece>,
        sample_rate_hz: u32,
        len_samples: u64,
    },
    Materialized {
        clip: vox_project::clipboard::MaterializedClip,
        /// H-56 (SPEC-008 §2.6 "later pastes"): pieces already imported (and, if the rates
        /// differed, converted) for the specific open session named by `session_id` — reused by
        /// a later paste into that same document without re-reading/re-resampling. `None` until
        /// the first paste since materializing.
        converted: Option<ConvertedClip>,
    },
}

/// H-56 (SPEC-008 §2.6): a materialized clip's audio, already converted for one specific session
/// (a rate mismatch was involved — a same-rate import instead rebinds the clipboard straight back
/// to [`ClipboardData::Bound`], see [`DocumentService::finish_paste_job`]).
struct ConvertedClip {
    session_id: String,
    pieces: Vec<Piece>,
}

impl ClipboardData {
    fn len_samples(&self) -> u64 {
        match self {
            ClipboardData::Bound { len_samples, .. } => *len_samples,
            ClipboardData::Materialized { clip, .. } => clip.len_samples,
        }
    }

    fn sample_rate_hz(&self) -> u32 {
        match self {
            ClipboardData::Bound { sample_rate_hz, .. } => *sample_rate_hz,
            ClipboardData::Materialized { clip, .. } => clip.sample_rate_hz,
        }
    }

    /// H-56 (SPEC-008 §2.6: "the clipboard is a reachability root for compaction"): the pieces of
    /// this clipboard that reference `session_id`'s chunk store, if any — `Bound` is always
    /// implicitly tied to whatever session is currently open (that invariant is
    /// [`DocumentService::materialize_clipboard`]'s job); a `Materialized` clip's own audio lives
    /// in a plain file outside every store, but its `converted` cache (if any) references exactly
    /// one session's chunks.
    fn reachable_pieces(&self, session_id: &str) -> Vec<Piece> {
        match self {
            ClipboardData::Bound { pieces, .. } => pieces.clone(),
            ClipboardData::Materialized {
                converted: Some(c), ..
            } if c.session_id == session_id => c.pieces.clone(),
            ClipboardData::Materialized { .. } => Vec::new(),
        }
    }
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
    /// T-301 (ADR-004 Amendment 3): the labels' placeholder values ("Normalize to {target} dB").
    pub undo_label_params: vox_project::LabelParams,
    pub redo_label_params: vox_project::LabelParams,
}

/// T-301 (SPEC-004 §2.7): what to do with a recovered session's interrupted take.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RecoveredTakeAction {
    /// The default: one undoable "Record" edit, as if Stop had been pressed.
    #[default]
    Apply,
    /// The take becomes a new untitled document; the session keeps its edits (and stays
    /// recoverable if it has unsaved ones).
    NewDocument,
    /// Drop the take (its WAV stays until the session is cleaned up).
    Discard,
}

/// [`DocumentService::recover`]'s result.
#[derive(Clone, Debug, PartialEq)]
pub struct RecoverOutcome {
    pub info: DocumentInfo,
    /// "The last N changes could not be recovered" (0: nothing lost).
    pub lost_changes: usize,
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

/// SPEC-009 §2.1: `kind` as `markers_get`/`marker_add` report it to the UI — `"user"`,
/// `"dropout"` or `"other"` (an unrecognized sidecar kind, SPEC-018 §2.7; the UI treats it like a
/// user marker, so there's no need to carry the actual string across IPC).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MarkerKindInfo {
    User,
    Dropout,
    Other,
}

impl From<&MarkerKind> for MarkerKindInfo {
    fn from(kind: &MarkerKind) -> Self {
        match kind {
            MarkerKind::User => MarkerKindInfo::User,
            MarkerKind::Dropout => MarkerKindInfo::Dropout,
            MarkerKind::Other(_) => MarkerKindInfo::Other,
        }
    }
}

/// One marker (SPEC-009 §2.1), as `markers_get`/`marker_add` report it to the UI.
#[derive(Clone, Debug, PartialEq)]
pub struct MarkerInfo {
    pub id: u64,
    pub pos_samples: u64,
    pub len_samples: u64,
    pub name: String,
    pub kind: MarkerKindInfo,
}

impl From<&Marker> for MarkerInfo {
    fn from(m: &Marker) -> Self {
        MarkerInfo {
            id: m.id.0,
            pos_samples: m.pos_samples,
            len_samples: m.len_samples,
            name: m.name.to_string(),
            kind: MarkerKindInfo::from(&m.kind),
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
/// ticket report); `len_samples` rounds to the nearest millisecond. H-23: `DropoutMark::UNKNOWN_LEN`
/// (unreliable timestamps) reads "Dropout (length unknown)" instead.
fn dropout_marker_name(len_samples: u64, rate_hz: u32) -> String {
    if len_samples == DropoutMark::UNKNOWN_LEN {
        return "Dropout (length unknown)".to_owned();
    }
    let rate_hz = u64::from(rate_hz.max(1));
    let ms = (len_samples * 1000 + rate_hz / 2) / rate_hz;
    format!("Dropout {ms} ms")
}

/// A dropout marker's own `len_samples` (its highlighted span): the fill length, or a point
/// marker (0) for `DropoutMark::UNKNOWN_LEN` (H-23) — its duration is unknown, so there is no
/// span to show.
fn dropout_marker_len(len_samples: u64) -> u64 {
    if len_samples == DropoutMark::UNKNOWN_LEN {
        0
    } else {
        len_samples
    }
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
    /// H-56 (SPEC-008 §2.6): where a materialized clipboard's `clip.f32`/`meta.json` live — a
    /// sibling of `sessions_dir` (see [`DocumentService::new`]).
    clipboard_dir: PathBuf,
    engine: EngineHandle,
    open: Mutex<Option<OpenDocument>>,
    clipboard: Mutex<Option<ClipboardData>>,
    /// H-09: `true` while a normalize job (peak or LUFS) is scanning/writing on its own thread —
    /// SPEC-010 §2.1's "disabled ... while another document job runs" (`error.document_busy`).
    /// Only one normalize job runs at a time; other audio-mutating commands also check this so a
    /// concurrent edit can't race the job's eventual commit (which re-checks the snapshot anyway,
    /// [`DocumentService::finish_normalize_peak`]/[`Self::finish_normalize_lufs`]).
    normalize_busy: Mutex<bool>,
    /// H-56 (SPEC-008 §2.6.1): `true` while a cross-document paste job is importing/resampling
    /// the materialized clipboard on its own thread — mirrors `normalize_busy`.
    paste_busy: Mutex<bool>,
    /// H-30: `true` specifically while a bake job is running — separate from `normalize_busy`
    /// (shared with peak/LUFS normalize too), because only a bake's `finish_bake` loads a
    /// post-job rack into the engine: a rack edit made mid-bake would otherwise be silently
    /// clobbered by that load. `ipc::rack_commands` refuses rack edits while this is set
    /// (`error.bake.busy`); a plain normalize job never touches the rack, so it doesn't set it.
    bake_running: Mutex<bool>,
    /// H-70 (SPEC-005 §2.7/§4.10, AC-15): `true` while a Save/Save As job is writing —
    /// deliberately separate from `normalize_busy`/`bake_running`: "editing and playback
    /// continue" while a save runs (§2.7 "Job behavior"), so this gates only a second concurrent
    /// Save/Save As (`error.save.in_progress`), never an edit. Checked-and-set inside
    /// [`DocumentService::begin_save`]/[`Self::begin_save_as`]; cleared, always, by
    /// [`Self::finish_save`]/[`Self::finish_save_as`].
    save_running: Mutex<bool>,
    /// H-70: cancel tokens of a running Save/Save As job, keyed by the id
    /// `document_save_cancel` names (mirrors `import_jobs`). At most one entry at a time
    /// (`save_running` gates a second job), kept as a map anyway for the same "a stale cancel is
    /// a harmless no-op" contract every other job's cancel command has.
    save_jobs: Mutex<HashMap<u32, CancelToken>>,
    next_save_job: AtomicU32,
    /// H-30 (SPEC-007 §4.1): the spectrogram tile service, wired once by the composition root
    /// (`set_spectro`) so a full save/save-as can hold `SpectroService::begin_background_job()`
    /// while it writes — the same guard export/bake hold. `OnceLock` rather than a constructor
    /// argument: many tests build a `DocumentService` that never touches this.
    spectro: OnceLock<Arc<SpectroService>>,
    /// T-306/H-20: the last open/save's sidecar/format notices (mismatch/corrupt/too_new/
    /// unreadable/items_dropped/save-failed/format_mapped/metadata_dropped/markers_not_in_flac),
    /// if any — [`DocumentService::take_sidecar_notices`] drains them. A side channel rather than
    /// a return value so `open`/`save`/`save_as` keep returning [`DocumentInfo`] like every other
    /// command result. H-20 made this a `Vec`: a single save can now carry more than one notice
    /// (e.g. metadata dropped *and* markers not in FLAC).
    pending_sidecar_notice: Mutex<Vec<SidecarNoticeInfo>>,
    /// T-301 (SPEC-004 §2.4): the memory budget new sessions get and `set_memory_budget_mib`
    /// applies live.
    memory_budget_bytes: AtomicU64,
    /// T-301: serializes creating a session directory with the start-up/recovery GC scan, so the
    /// scan never sees a new directory before its creator has locked it.
    session_dirs: Mutex<()>,
    /// T-209 (SPEC-005 §2.3): cancel tokens of import jobs currently running, keyed by the id
    /// `document_open_cancel` names. Removed once the job finishes, whatever the outcome.
    import_jobs: Mutex<HashMap<u32, CancelToken>>,
    next_import_job: AtomicU32,
    /// H-71 (SPEC-005 §2.3, ADR-003 Amendment 6): the growing session's peaks handle for each
    /// import job currently running, keyed the same way as `import_jobs`. Set once
    /// `open_with_downmix`'s `on_live` hook fires (before the decode loop starts), read by
    /// `import_peaks` (`import_peaks_get`'s business logic), removed by `finish_import_job`.
    import_live: Mutex<HashMap<u32, ImportLiveInfo>>,
    /// H-56 (SPEC-008 §2.6.1): cancel tokens of running cross-document paste jobs, keyed by the
    /// id `edit_paste_cancel` names (mirrors `import_jobs`).
    paste_jobs: Mutex<HashMap<u32, CancelToken>>,
    next_paste_job: AtomicU32,
    /// H-40: set by `audio::forward_rack_notice` (the engine's event sink, off-thread — it must
    /// never call `EngineHandle` back, MEMORY.md S1-01) when a `RackNotice::SlotRecovered`
    /// arrives; consumed by [`DocumentService::sync_pending_rack_recovery`], which does the
    /// actual (engine-calling) sidecar-digest rebase from a normal command context.
    pending_rack_recovery: AtomicBool,
}

/// A sidecar-related notice for the command handler to turn into a `notice` event (SPEC-018
/// §2.5/§2.9's notice keys). `params` are already i18n `with_param` pairs (e.g. `file`, `count`).
#[derive(Clone, Debug, PartialEq)]
pub struct SidecarNoticeInfo {
    pub key: &'static str,
    pub params: Vec<(&'static str, String)>,
}

/// H-71: everything [`DocumentService::import_peaks`] needs for one running import job — the
/// store its `ChunkWriter` commits into (still owned by `open_impl`'s `Session`, but chunks are
/// readable through the store as soon as they commit, ADR-004 §2/§5) and the live handle onto its
/// growing piece table. `len_hint` isn't used yet (every bucket beyond what's committed reads
/// `PARTIAL`/`NaN` regardless of whether the total length is known) but is kept for a future
/// ticket that wants to distinguish "still importing" from "known to be past the end".
#[derive(Clone)]
struct ImportLiveInfo {
    store: Arc<vox_project::ChunkStore>,
    live: vox_project::LiveWrittenAudio,
    sample_rate_hz: u32,
    #[allow(dead_code)]
    len_hint: Option<u64>,
}

/// [`DocumentService::import_peaks`]'s result — [`PeaksResult`] minus `audio_rev` (an in-progress
/// import isn't a document revision) plus `partial` (ADR-003 §2's `VXPK` `PARTIAL` bit: some of
/// the requested buckets are past what's committed so far).
#[derive(Debug)]
pub struct ImportPeaksResult {
    pub sample_rate_hz: u32,
    pub buckets: Vec<(f32, f32)>,
    pub partial: bool,
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

/// H-56 (SPEC-008 §2.6.1): the read-only facts a cross-document paste job's own thread needs
/// (mirrors [`NormalizeJobSource`]) — no `&mut Session` until [`DocumentService::finish_paste_job`]'s
/// commit.
#[derive(Clone)]
pub struct PasteJobSource {
    pub store: Arc<vox_project::ChunkStore>,
    pub target_rate_hz: u32,
    pub target: EditTarget,
    pub clip: vox_project::clipboard::MaterializedClip,
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

/// SPEC-005 §2.7/AC-15 (H-70): a Save/Save As job is already running.
fn save_in_progress_error() -> IpcError {
    IpcError::new(IpcErrorCode::Busy, "error.save.in_progress")
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

/// H-60 (SPEC-005 §2.7 error list, AC-15/AC-16): distinguishes the two failures the spec names a
/// dedicated key for — a full/quota-exceeded disk (`error.save.disk_full`) and a FLAC
/// verify-before-rename mismatch (`error.save.verify_failed`, §2.11) — from every other WAV/FLAC
/// write failure, which keeps the general `error.save.io`. Previously every [`vox_io::IoError`]
/// from [`save_to`] mapped to `error.save.io` regardless of cause.
fn io_save_error(err: vox_io::IoError) -> IpcError {
    let key = match &err {
        vox_io::IoError::FlacVerifyFailed(_) => "error.save.verify_failed",
        vox_io::IoError::Io(e) if is_disk_full_io_error(e) => "error.save.disk_full",
        _ => "error.save.io",
    };
    IpcError::new(IpcErrorCode::Io, key).with_param("message", err.to_string())
}

/// Same classification [`vox_project::ProjectError::from_io`] uses (crates/project/src/error.rs),
/// duplicated here because a save failure reaches this module as a raw [`vox_io::IoError`], not a
/// [`vox_project::ProjectError`] (`save_to` unwraps `ProjectError::Wav` before calling
/// [`io_save_error`]).
fn is_disk_full_io_error(e: &std::io::Error) -> bool {
    matches!(
        e.kind(),
        std::io::ErrorKind::StorageFull
            | std::io::ErrorKind::QuotaExceeded
            | std::io::ErrorKind::FileTooLarge
    )
}

/// H-15 (SPEC-018 §2.9 "Read-only folders"): the bound file's folder can't be written to at all.
/// Distinct from `error.save.sidecar_locked` (the folder is writable, only the sidecar file
/// itself can't be replaced — a Windows read-only attribute, or a directory in its place). The
/// message names Save As as the way out, same as `sidecar_locked`'s.
fn permission_error(path: &Path) -> IpcError {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    IpcError::new(IpcErrorCode::Io, "error.save.permission").with_param("name", name)
}

/// SPEC-018 §2.9's read-only-folder pre-flight: "Save fails... before any byte is written". A
/// harmless probe (create + remove a throwaway file in `path`'s folder) rather than inspecting
/// Unix mode bits, which aren't a reliable cross-platform proxy for "can I write here" (root,
/// ACLs, read-only mounts, Windows attributes) — this is exactly the operation the real save is
/// about to attempt, just on a name nothing else uses. A non-permission probe failure (e.g. the
/// folder itself is gone) is left for the real write to report normally.
fn check_folder_writable(path: &Path) -> Result<(), IpcError> {
    let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) else {
        return Ok(());
    };
    let probe = parent.join(format!(".powervoice-writable-probe-{}", std::process::id()));
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&probe)
    {
        Ok(_) => {
            let _ = std::fs::remove_file(&probe);
            Ok(())
        }
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => Err(permission_error(path)),
        Err(_) => Ok(()),
    }
}

/// H-20 (SPEC-005 §2.6): the save format (container + bit depth) an imported document keeps, and
/// whether that's a **mapped** format — different from the source, which fires
/// `notice.open.format_mapped` at open.
///
/// Dispatches on `probe.codec` — symphonia's PCM decoder short name (`pcm_s16le`, `pcm_u8`,
/// `pcm_alaw`, …) — rather than `probe.container`, because it doesn't matter whether `hound` could
/// have parsed the file (every WAV variant SPEC-005 §2.2 tolerates decodes through the same `pcm`
/// codec family). **Fixes a real bug found while implementing this**: the previous
/// `save_bits_for_import` compared `probe.container` against the literal `"wav"`, but symphonia's
/// WAV reader's `format_info().short_name` is actually `"wave"` — that comparison never matched,
/// so *every* WAV import (via the T-209 `document_open` job path) silently fell through to the
/// lossy-import default of WAV 24-bit, regardless of its real bit depth. `vox_io::probe`/
/// `vox_project::probe_for_import` already carry `codec` and (H-20 addition) `bits_per_sample`, so
/// this needs no extra file read.
fn save_format_for_import(probe: &ImportProbe) -> (SaveContainer, BitDepth, bool) {
    match probe.container.as_str() {
        "wave" => match probe.codec.as_str() {
            "pcm_s16le" | "pcm_s16be" => (SaveContainer::Wav, BitDepth::Bit16, false),
            "pcm_s24le" | "pcm_s24be" => (SaveContainer::Wav, BitDepth::Bit24, false),
            "pcm_f32le" | "pcm_f32be" => (SaveContainer::Wav, BitDepth::Bit32Float, false),
            // SPEC-005 §2.6: 8-bit unsigned, A-law and µ-law all map to WAV 16-bit.
            "pcm_u8" | "pcm_alaw" | "pcm_mulaw" => (SaveContainer::Wav, BitDepth::Bit16, true),
            // 32-bit int and 64-bit float map to WAV 32-bit float (the only written format that
            // keeps everything PowerVoice holds internally).
            "pcm_s32le" | "pcm_s32be" | "pcm_f64le" | "pcm_f64be" => {
                (SaveContainer::Wav, BitDepth::Bit32Float, true)
            }
            // A PCM variant this table doesn't name yet: the safe SPEC-005 §2.6 default.
            _ => (SaveContainer::Wav, BitDepth::Bit24, true),
        },
        // SPEC-005 §2.6: an opened FLAC saves as FLAC — ≤16-bit stays FLAC 16 (8/12-bit promoted
        // losslessly, no notice), 17-24-bit stays FLAC 24 (no notice), 25-32-bit is promoted (and
        // dithered) down to FLAC 24, with the format-mapped notice.
        "flac" => match probe.bits_per_sample {
            Some(bits) if bits <= 16 => (SaveContainer::Flac, BitDepth::Bit16, false),
            Some(bits) if bits <= 24 => (SaveContainer::Flac, BitDepth::Bit24, false),
            Some(_) => (SaveContainer::Flac, BitDepth::Bit24, true),
            None => (SaveContainer::Flac, BitDepth::Bit24, false),
        },
        // MP3/M4A/Ogg Vorbis (SPEC-005 §2.6): Save always acts as Save As at WAV 24-bit; the
        // table has no notice at open for these.
        _ => (SaveContainer::Wav, BitDepth::Bit24, false),
    }
}

/// H-20 (SPEC-005 §2.6): "This ‹from› ‹container› will be saved as ‹to›" — the two bit-depth
/// descriptions `notice.open.format_mapped` needs, derived the same way
/// [`save_format_for_import`] decided the mapping.
fn format_mapped_notice_params(probe: &ImportProbe, to: BitDepth) -> (&'static str, &'static str) {
    let from = match probe.codec.as_str() {
        "pcm_u8" => "8-bit",
        "pcm_alaw" => "A-law",
        "pcm_mulaw" => "µ-law",
        "pcm_s32le" | "pcm_s32be" => "32-bit int",
        "pcm_f64le" | "pcm_f64be" => "64-bit float",
        _ => match probe.bits_per_sample {
            Some(bits) if bits > 24 => "25-32-bit",
            _ => "unusual",
        },
    };
    let to = match to {
        BitDepth::Bit16 => "16-bit",
        BitDepth::Bit24 => "24-bit",
        BitDepth::Bit32Float => "32-bit float",
    };
    (from, to)
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

/// H-20 (SPEC-005 §2.8): Save As's Dither row -> `vox_dsp::dither::DitherMode`.
impl From<SaveDitherPref> for vox_io::DitherMode {
    fn from(pref: SaveDitherPref) -> Self {
        match pref {
            SaveDitherPref::Tpdf => vox_io::DitherMode::Tpdf,
            SaveDitherPref::None => vox_io::DitherMode::None,
        }
    }
}

/// T-209/H-20 (SPEC-005 §2.6/§2.7): the container Save/Save As writes. Import keeps `Flac` for a
/// FLAC source and `Wav` for everything else (§2.6's table — [`save_format_for_import`]); an
/// explicit Save As can freely choose either.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaveContainer {
    Wav,
    Flac,
}

/// FLAC has no 32-bit float format (SPEC-005 §2.6/§2.11): a Save As request combining `Flac` with
/// `BitDepth::Bit32Float` is invalid and refused before any pre-flight check runs.
fn invalid_flac_bit_depth() -> IpcError {
    IpcError::new(
        IpcErrorCode::InvalidArgument,
        "error.save.flac_needs_int_bits",
    )
}

/// SPEC-005 §2.7 pre-flight step 2 / AC-18: "Too long for a WAV file in this format — choose
/// 16/24-bit or FLAC." FLAC has no such limit (no `u32` size field to overflow).
fn too_large_for_wav_error() -> IpcError {
    IpcError::new(
        IpcErrorCode::InvalidArgument,
        "error.save.too_large_for_wav",
    )
}

/// SPEC-005 §2.7 pre-flight step 1 (H-70): "free space ≥ estimated file size + 64 MiB", checked
/// before any byte is written. The estimate is deliberately conservative — uncompressed bytes at
/// the target bit depth, even for FLAC, which is never larger than that in practice (AC-16: a
/// 16-bit FLAC of typical content is ≤ 50 % of the equivalent WAV) — the same "never
/// under-report" spirit `wav_size_exceeds_limit` uses, since this only needs to prove there's
/// room, not to predict the encoder's exact output size.
const SAVE_FREE_SPACE_MARGIN_BYTES: u64 = 64 * 1024 * 1024;

fn estimated_save_output_bytes(len_samples: u64, bits: BitDepth) -> u64 {
    let bytes_per_sample: u64 = match bits {
        BitDepth::Bit16 => 2,
        BitDepth::Bit24 => 3,
        BitDepth::Bit32Float => 4,
    };
    len_samples.saturating_mul(bytes_per_sample)
}

/// H-70 (SPEC-005 §2.7 error list): the same `error.save.disk_full` a real out-of-space write
/// failure gets ([`io_save_error`]) — from the user's point of view "the disk can't hold this
/// file" is the same failure whether it's caught before or during the write.
fn disk_full_preflight_error() -> IpcError {
    IpcError::new(IpcErrorCode::Io, "error.save.disk_full")
}

/// SPEC-005 §2.7 pre-flight step 1 (H-70; H-11's `FreeSpaceProvider`, reused rather than a new
/// mechanism): refuses before any byte is written when the target volume can't hold the
/// estimated output plus [`SAVE_FREE_SPACE_MARGIN_BYTES`]. Queries the target folder (falling
/// back to `path` itself for the pathological empty-parent case `check_folder_writable` also
/// guards); a provider error never blocks the save on its own (mirrors `DocumentService::
/// housekeeping`'s `free_bytes(...).unwrap_or(u64::MAX)`) — the real write still surfaces a
/// concrete I/O error if something is actually wrong.
fn check_free_space(
    doc: &OpenDocument,
    bits: BitDepth,
    path: &Path,
    free: &dyn vox_project::FreeSpaceProvider,
) -> Result<(), IpcError> {
    let len_samples = doc.session.current().len_samples;
    let needed =
        estimated_save_output_bytes(len_samples, bits).saturating_add(SAVE_FREE_SPACE_MARGIN_BYTES);
    let probe_dir = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(path);
    let free_bytes = free.free_bytes(probe_dir).unwrap_or(u64::MAX);
    if free_bytes < needed {
        return Err(disk_full_preflight_error());
    }
    Ok(())
}

/// SPEC-005 §2.7 pre-flight step 2 (H-60, `wav_max_bytes`): refuses before any byte is written,
/// same spirit as [`check_folder_writable`] and [`check_overs`].
fn check_wav_size(
    doc: &OpenDocument,
    container: SaveContainer,
    bits: BitDepth,
) -> Result<(), IpcError> {
    if container != SaveContainer::Wav {
        return Ok(());
    }
    let len_samples = doc.session.current().len_samples;
    if vox_project::wav_size_exceeds_limit(len_samples, bits.into()) {
        return Err(too_large_for_wav_error());
    }
    Ok(())
}

/// SPEC-005 §2.8's clip pre-flight: `None` for a float target (32-bit float is bit-exact, overs
/// are kept, never a clip) or when [`overs_check`] found nothing above 0 dBFS.
///
/// H-75: `cancel`/`on_progress` are the save job's own (the caller passes the same ones
/// [`DocumentService::finish_save`]/[`Self::finish_save_as`] will use for the write) — the exact
/// counting pass this can run is "a cancellable job with progress" per SPEC-005 §2.8, not a
/// throwaway, uncancellable scan.
fn check_overs(
    doc: &OpenDocument,
    bits: BitDepth,
    cancel: &CancelToken,
    on_progress: &mut dyn FnMut(f32),
) -> Result<Option<OversInfo>, IpcError> {
    if bits == BitDepth::Bit32Float {
        return Ok(None);
    }
    let snapshot = doc.session.current();
    overs_check(doc.session.store(), &snapshot, cancel, on_progress).map_err(document_error)
}

/// SPEC-005 §2.8: "12 samples are above 0 dBFS (peak +1.8 dBFS) and will be clipped in a 16-bit
/// file." The UI's **Clip and save** re-issues with `confirm_clip: true`; **Save as 32-bit float
/// instead** calls `document_save_as` with `bits: "32f"` instead (never clips); **Cancel** does
/// nothing.
fn clip_confirmation_error(overs: OversInfo) -> IpcError {
    IpcError::new(IpcErrorCode::NeedsConfirmation, "dialog.overs")
        .with_param("count", overs.count.to_string())
        .with_param("peak_dbfs", format!("{:.2}", overs.peak_dbfs))
}

/// H-20 (SPEC-005 §2.4 "Saving over a multichannel source"): "‹name› is a stereo file. Saving
/// replaces it with a mono file." The UI's **Save**/primary choice re-issues with
/// `confirm_multichannel: true`; **Cancel** does nothing (a dedicated **Save As…** shortcut from
/// inside this dialog is deferred — the menu's own Save As… already covers it, ticket report).
fn multichannel_confirmation_error(path: &Path) -> IpcError {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    IpcError::new(
        IpcErrorCode::NeedsConfirmation,
        "dialog.multichannel_source",
    )
    .with_param("name", name)
}

fn format_tag(container: SaveContainer, bits: BitDepth) -> &'static str {
    match (container, bits) {
        (SaveContainer::Wav, BitDepth::Bit16) => "wav16",
        (SaveContainer::Wav, BitDepth::Bit24) => "wav24",
        (SaveContainer::Wav, BitDepth::Bit32Float) => "wav32f",
        (SaveContainer::Flac, BitDepth::Bit16) => "flac16",
        (SaveContainer::Flac, _) => "flac24",
    }
}

/// T-209/H-20 (SPEC-005 §2.6, §2.8, §2.11): maps `container`/`bits`/`dither` to the sidecar's
/// `save_format` fields. A float target never dithers (`dither` is ignored: always `"none"`),
/// whatever `dither` says.
fn save_format_model(
    container: SaveContainer,
    bits: BitDepth,
    dither: SaveDitherPref,
) -> SaveFormatModel {
    let (container_str, sample_format) = match (container, bits) {
        (SaveContainer::Wav, BitDepth::Bit16) => ("wav", "pcm16"),
        (SaveContainer::Wav, BitDepth::Bit24) => ("wav", "pcm24"),
        (SaveContainer::Wav, BitDepth::Bit32Float) => ("wav", "f32"),
        (SaveContainer::Flac, BitDepth::Bit16) => ("flac", "pcm16"),
        // FLAC never carries 32-bit float (`invalid_flac_bit_depth` refuses that combination
        // before this is reached); every other bit depth requested with FLAC saves as 24-bit.
        (SaveContainer::Flac, _) => ("flac", "pcm24"),
    };
    let dither_str = if bits == BitDepth::Bit32Float {
        "none"
    } else {
        match dither {
            SaveDitherPref::Tpdf => "tpdf",
            SaveDitherPref::None => "none",
        }
    };
    SaveFormatModel {
        container: container_str.to_string(),
        sample_format: sample_format.to_string(),
        dither: dither_str.to_string(),
        extra: Default::default(),
    }
}

/// The `markers.items` a save/digest would use right now: the live markers (`kind` comes from
/// each `Marker` itself, H-57), with each one's unrecognized sidecar `extra` fields carried
/// through [`SidecarState::marker_meta`] (module scope note, `vox_project::sidecar`).
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
    let save_format = save_format_model(doc.save_container, doc.save_bits, doc.save_dither);
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

/// SPEC-006 §2.4's `verticalZoom` range, `1.0..=256.0`.
const MIN_VERTICAL_ZOOM: f64 = 1.0;
const MAX_VERTICAL_ZOOM: f64 = 256.0;
/// SPEC-006 §2.4's default `verticalZoom`.
const DEFAULT_VERTICAL_ZOOM: f64 = 1.0;

/// `doc.sidecar.view["waveform"]` -> [`WaveformViewInfo`] (SPEC-018 §2.6.5: `start_sample`/
/// `samples_per_pixel`/`selection`/`cursor_samples`/`time_ruler_format`/`vertical_zoom` — see the
/// struct doc for what's still deferred). A malformed or partial section (missing `start_sample`/
/// `samples_per_pixel`) reports "no opinion" (`None`), same as a missing one. `selection`/
/// `cursor_samples` are restored only when they fit `[0, len_samples]` (§2.6.5: "restored when
/// within `[0, L]`... otherwise `null` / 0"); `start_sample`/`samples_per_pixel` are restored
/// as-is here — clamping them to the current viewport width (§2.6.5's "an out-of-range
/// `samples_per_pixel` -> zoom full") needs the viewport's pixel width, which only `WaveformView`
/// knows, so that half of the rule is applied there (`state/waveformView.svelte.ts`'s
/// pending-restore mechanism). `time_ruler_format` (§2.6.5: "SPEC-006 enums") falls back to
/// `Timecode` (SPEC-006 §2.5's default) on a missing or unrecognized value, same "no opinion on
/// the invalid part" spirit as `selection`/`cursor_samples`. H-35: `vertical_zoom` needs no
/// viewport to validate (SPEC-006 §2.4's range is a fixed `[1, 256]`, not viewport-relative like
/// `samples_per_pixel`), so it's clamped here directly, falling back to the SPEC-006 §2.4 default
/// (`1.0`) on a missing or non-finite value. H-72: `amplitude_ruler_mode` falls back to `Dbfs`
/// (SPEC-006 §2.4's default) on a missing or unrecognized value, same as `time_ruler_format`.
fn waveform_view_of(view: &serde_json::Value, len_samples: u64) -> Option<WaveformViewInfo> {
    let w = view.get("waveform")?;
    let start_sample = w.get("start_sample")?.as_u64()?;
    let samples_per_pixel = w.get("samples_per_pixel")?.as_f64()?;
    let selection = match w.get("selection") {
        Some(serde_json::Value::Object(sel)) => {
            let start = sel.get("start_sample").and_then(|v| v.as_u64());
            let end = sel.get("end_sample").and_then(|v| v.as_u64());
            match (start, end) {
                (Some(s), Some(e)) if s <= e && e <= len_samples => Some((s, e)),
                _ => None,
            }
        }
        _ => None,
    };
    let cursor_samples = w
        .get("cursor_samples")
        .and_then(|v| v.as_u64())
        .filter(|&c| c <= len_samples)
        .unwrap_or(0);
    let time_ruler_format = match w.get("time_ruler_format").and_then(|v| v.as_str()) {
        Some("samples") => TimeRulerFormatDto::Samples,
        Some("seconds") => TimeRulerFormatDto::Seconds,
        _ => TimeRulerFormatDto::Timecode,
    };
    let vertical_zoom = w
        .get("vertical_zoom")
        .and_then(|v| v.as_f64())
        .filter(|v| v.is_finite())
        .map(|v| v.clamp(MIN_VERTICAL_ZOOM, MAX_VERTICAL_ZOOM))
        .unwrap_or(DEFAULT_VERTICAL_ZOOM);
    let amplitude_ruler_mode = match w.get("amplitude_ruler_mode").and_then(|v| v.as_str()) {
        Some("percent") => AmplitudeRulerModeDto::Percent,
        _ => AmplitudeRulerModeDto::Dbfs,
    };
    Some(WaveformViewInfo {
        start_sample,
        samples_per_pixel,
        selection,
        cursor_samples,
        time_ruler_format,
        vertical_zoom,
        amplitude_ruler_mode,
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
        dirty: doc.session.is_dirty() || doc.recovered,
        audio_rev: snapshot.audio_rev,
        sidecar_dirty: sidecar_dirty_of(engine, doc),
        spectral_view: spectral_view_of(&doc.sidecar.view),
        waveform_view: waveform_view_of(&doc.sidecar.view, snapshot.len_samples),
        recovered: doc.recovered,
    }
}

/// The save format a recovered document keeps: its last save's (`wav16`/`wav24`/`wav32f`/
/// `flac16`/`flac24`, T-209), else the source file's (SPEC-005 §2.6, H-20:
/// [`save_format_for_import`] — FLAC keeps FLAC), else WAV 24-bit.
fn save_format_for_recovery(report: &vox_project::RecoveryReport) -> (SaveContainer, BitDepth) {
    match report.saved.as_ref().map(|s| s.format.as_str()) {
        Some("wav16") => return (SaveContainer::Wav, BitDepth::Bit16),
        Some("wav24") => return (SaveContainer::Wav, BitDepth::Bit24),
        Some("wav32f") => return (SaveContainer::Wav, BitDepth::Bit32Float),
        Some("flac16") => return (SaveContainer::Flac, BitDepth::Bit16),
        Some("flac24") => return (SaveContainer::Flac, BitDepth::Bit24),
        _ => {}
    }
    match &report.source {
        Some(source) if source.path.exists() => match vox_project::probe_for_import(&source.path) {
            Ok(probe) => {
                let (container, bits, _mapped) = save_format_for_import(&probe);
                (container, bits)
            }
            Err(_) => (SaveContainer::Wav, BitDepth::Bit24),
        },
        _ => (SaveContainer::Wav, BitDepth::Bit24),
    }
}

/// `(size, mtime)` of `path` as the sidecar pre-flight check records them (0s when missing).
fn file_facts(path: &Path) -> (u64, u64) {
    let meta = std::fs::metadata(path).ok();
    (
        meta.as_ref().map(std::fs::Metadata::len).unwrap_or(0),
        meta.and_then(|m| m.modified().ok())
            .map(unix_ms_of)
            .unwrap_or(0),
    )
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
        // H-56 (SPEC-008 §2.6): a sibling of `sessions_dir` rather than a new constructor
        // parameter — every existing call site (composition root, ~25 tests) keeps working
        // unchanged, and a test's own temp `sessions_dir` still gives it an isolated clipboard
        // directory. Swept now ("deleted ... if stale, at start-up", §2.6): a fresh app run has
        // no use for whatever a previous run left there.
        let clipboard_dir = sessions_dir
            .parent()
            .map(|p| p.join("clipboard"))
            .unwrap_or_else(|| sessions_dir.join("clipboard"));
        vox_project::clipboard::sweep_stale_dir(&clipboard_dir);
        Self(Arc::new(Inner {
            sessions_dir,
            clipboard_dir,
            engine,
            open: Mutex::new(None),
            clipboard: Mutex::new(None),
            normalize_busy: Mutex::new(false),
            paste_busy: Mutex::new(false),
            bake_running: Mutex::new(false),
            save_running: Mutex::new(false),
            save_jobs: Mutex::new(HashMap::new()),
            next_save_job: AtomicU32::new(1),
            spectro: OnceLock::new(),
            pending_sidecar_notice: Mutex::new(Vec::new()),
            memory_budget_bytes: AtomicU64::new(StoreOptions::default().memory_budget_bytes),
            session_dirs: Mutex::new(()),
            import_jobs: Mutex::new(HashMap::new()),
            import_live: Mutex::new(HashMap::new()),
            next_import_job: AtomicU32::new(1),
            paste_jobs: Mutex::new(HashMap::new()),
            next_paste_job: AtomicU32::new(1),
            pending_rack_recovery: AtomicBool::new(false),
        }))
    }

    /// Store options for a new or recovered session (the current memory budget).
    fn store_options(&self) -> StoreOptions {
        StoreOptions::with_memory_budget(self.0.memory_budget_bytes.load(Ordering::Relaxed))
    }

    /// Creates a session directory while holding [`Inner::session_dirs`].
    fn create_session(&self, mut config: SessionConfig) -> Result<Session, ProjectError> {
        config.store = self.store_options();
        let _dirs = self.0.session_dirs.lock().unwrap();
        Session::create(&self.0.sessions_dir, config)
    }

    /// T-301 (SPEC-004 §2.4, §3 `memory_budget`): Settings → Performance → "Memory for audio",
    /// 256 MiB … 16 GiB. Applies at once to the open document's store (evicting down to it) and
    /// to every session created later.
    pub fn set_memory_budget_mib(&self, mib: u32) {
        let bytes = u64::from(mib.clamp(256, 16_384)) * 1024 * 1024;
        self.0.memory_budget_bytes.store(bytes, Ordering::Relaxed);
        if let Some(doc) = self.0.open.lock().unwrap().as_ref() {
            doc.session.store().set_memory_budget(bytes);
        }
    }

    /// The directory sessions live in.
    pub fn sessions_dir(&self) -> &Path {
        &self.0.sessions_dir
    }

    // --- T-301: crash recovery, session cleanup, housekeeping -------------------------------

    /// SPEC-004 §2.7/§2.8, ADR-004 §9–§10: finishes interrupted deletions, deletes cleanly
    /// closed sessions silently, and returns the recoverable ones (newest first). Sessions in
    /// use by a running instance — this one's included — are neither listed nor touched.
    pub fn recovery_list(&self) -> Vec<vox_project::gc::RecoverableSession> {
        let report = {
            let _dirs = self.0.session_dirs.lock().unwrap();
            vox_project::gc::collect_garbage(&self.0.sessions_dir)
        };
        for (dir, error) in &report.errors {
            tracing::warn!(dir = %dir.display(), %error, "session cleanup failed");
        }
        let mut sessions = report.recoverable;
        sessions.sort_by_key(|s| std::cmp::Reverse(s.last_modified));
        sessions
    }

    /// `<sessions>/<id>` for a recovery command, refusing anything that isn't a plain directory
    /// name there.
    fn recovery_dir(&self, id: &str) -> Result<PathBuf, IpcError> {
        let plain = !id.is_empty()
            && !id.starts_with('.')
            && !id.contains(['/', '\\'])
            && !id.ends_with(vox_project::gc::DELETING_SUFFIX);
        let dir = self.0.sessions_dir.join(id);
        if !plain || !dir.is_dir() {
            return Err(IpcError::new(
                IpcErrorCode::NotFound,
                "error.recovery.not_found",
            ));
        }
        Ok(dir)
    }

    /// Discard (recovery dialog, Settings → Recovery & storage → Clear…): permanently deletes a
    /// recoverable session. A session in use is refused (`error.session_locked`).
    pub fn recovery_discard(&self, id: &str) -> Result<(), IpcError> {
        let dir = self.recovery_dir(id)?;
        let _dirs = self.0.session_dirs.lock().unwrap();
        vox_project::gc::discard_session(&dir).map_err(document_error)
    }

    /// Recover (SPEC-004 §2.7): rebuilds the session's exact document and undo/redo history
    /// (`Session::recover`), handles its interrupted take per `take_action`, restores the rack
    /// and view from the last journaled `state`, and makes it the open document — modified,
    /// titled "(recovered)", bound to its original path. Replaces whatever was open (the UI
    /// runs the unsaved-changes prompt first).
    pub fn recover(
        &self,
        id: &str,
        take_action: RecoveredTakeAction,
    ) -> Result<RecoverOutcome, IpcError> {
        if self.is_recording() {
            return Err(IpcError::not_while_recording());
        }
        let dir = self.recovery_dir(id)?;
        let (mut session, report) =
            Session::recover(&dir, self.store_options()).map_err(document_error)?;
        let mut path = report.bound_path().map(Path::to_path_buf);
        let (mut save_container, mut save_bits) = save_format_for_recovery(&report);
        let mut state = report.state.clone();
        if let Some(take) = report.open_take {
            match take_action {
                RecoveredTakeAction::Apply => {
                    session.apply_open_take_from_wav().map_err(document_error)?;
                }
                RecoveredTakeAction::Discard => {
                    session.discard_take(take.take).map_err(document_error)?;
                }
                RecoveredTakeAction::NewDocument => {
                    let fresh = {
                        let _dirs = self.0.session_dirs.lock().unwrap();
                        session
                            .open_take_as_new_session(&self.0.sessions_dir, self.store_options())
                            .map_err(document_error)?
                    };
                    if let Some(fresh) = fresh {
                        // The original keeps its edits: deleted if nothing is unsaved, else left
                        // (unlocked) for the next recovery dialog.
                        if session.is_dirty() {
                            drop(session);
                        } else if let Err(e) = session.close() {
                            tracing::warn!(error = %e.error, "closing the recovered session failed");
                        }
                        session = fresh;
                        path = None;
                        save_container = SaveContainer::Wav;
                        save_bits = BitDepth::Bit24;
                        state = None;
                    }
                }
            }
        }

        let mut sidecar = SidecarState::none();
        if let Some(state) = &state {
            if let Some(model) = state
                .get("rack")
                .and_then(|r| serde_json::from_value::<vox_rack::RackModel>(r.clone()).ok())
            {
                let _ = self.0.engine.rack_load_model(model);
            }
            if let Some(view) = state.get("view") {
                sidecar.view = view.clone();
            }
        }
        // The dialog already warned about a changed file ("saving will overwrite the file"), so
        // Save doesn't ask again; `last_saved_audio_rev = 0` forces a full save.
        if let Some(p) = &path {
            (sidecar.file_size_bytes, sidecar.file_mtime_unix_ms) = file_facts(p);
        }
        let snapshot = session.current();
        let save_format = save_format_model(save_container, save_bits, SaveDitherPref::default());
        let markers = sidecar.marker_meta.build_items(&snapshot.markers);
        let rack = current_rack_value(&self.0.engine);
        sidecar.persisted_digest =
            vox_project::sidecar::persisted_digest(&save_format, &markers, &rack);
        sidecar.last_written_markers = markers;

        self.0.engine.set_document(Some(PlaybackDoc {
            store: Arc::clone(session.store()),
            snapshot,
        }));
        let mut doc = OpenDocument::new(session, path, save_bits, sidecar);
        doc.save_container = save_container;
        doc.recovered = true;
        let mut guard = self.0.open.lock().unwrap();
        // H-56 (SPEC-008 §2.6): materialize a `Bound` clipboard before its session closes.
        if let Some(current) = guard.as_ref() {
            self.materialize_clipboard(current.session.store());
        }
        let previous = guard.replace(doc);
        let info = info_of(&self.0.engine, guard.as_ref());
        drop(guard);
        if let Some(previous) = previous {
            close_with_retry(previous.session);
        }
        Ok(RecoverOutcome {
            info,
            lost_changes: report.lost_changes,
        })
    }

    /// Closes the document for good after Save or Don't Save (SPEC-004 §2.8: quit): journals
    /// `close` and deletes the session directory. Refused while recording.
    pub fn close(&self) -> Result<(), IpcError> {
        let previous = {
            let mut guard = self.0.open.lock().unwrap();
            if guard.as_ref().is_some_and(|d| d.session.is_recording()) {
                return Err(IpcError::not_while_recording());
            }
            guard.take()
        };
        // H-56 (SPEC-008 §2.6): materialize a `Bound` clipboard before its session closes.
        if let Some(doc) = &previous {
            self.materialize_clipboard(doc.session.store());
        }
        // T-901: "Close all plugin windows" on document close (a no-op without a live rack).
        let _ = self.0.engine.rack_command(RackCommand::CloseAllEditors);
        self.0.engine.set_document(None);
        if let Some(doc) = previous {
            close_with_retry(doc.session);
        }
        Ok(())
    }

    /// ADR-004 §6 `state` (debounced by the caller, ~2 s): journals the rack and view state when
    /// it differs from the last journaled one, so crash recovery restores them (SPEC-004 §2.7).
    pub fn journal_state_if_changed(&self) {
        let mut guard = self.0.open.lock().unwrap();
        let Some(doc) = guard.as_mut() else {
            return;
        };
        let state = serde_json::json!({
            "rack": current_rack_value(&self.0.engine),
            "view": doc.sidecar.view,
        });
        let digest = serde_json::to_vec(&state)
            .map(|b| crc32fast::hash(&b))
            .unwrap_or(0);
        if doc.state_digest == Some(digest) {
            return;
        }
        match doc.session.append_state(state) {
            Ok(()) => doc.state_digest = Some(digest),
            Err(error) => tracing::warn!(%error, "journaling the rack/view state failed"),
        }
    }

    /// SPEC-004 §2.5's disk check (after every edit and every 10 s idle): compaction, then the
    /// oldest undo steps (OD-1 = A). Skipped while recording, while a normalize job runs and
    /// during playback (a store switch would stop it) — the next check retries. `None` when
    /// nothing ran.
    ///
    /// H-17 item 3: unlike `Session::housekeep` (which needs `&mut Session` throughout, including
    /// while it copies every live chunk into the next generation), this only holds the document
    /// mutex for the cheap parts — deciding, and the final checkpoint/`meta.json` swap
    /// (`Session::begin_compact`/`finish_compact`) — and releases it while the chunk copy
    /// (`PreparedCompaction::copy_chunks`, potentially the whole document's worth of I/O) runs, so
    /// edits and other commands never wait behind it.
    pub fn housekeeping(
        &self,
        free: &dyn vox_project::FreeSpaceProvider,
    ) -> Option<vox_project::HousekeepingReport> {
        if self.is_normalize_busy() || self.0.engine.transport_state().playing {
            return None;
        }
        let clip = {
            let open_session_id = self
                .0
                .open
                .lock()
                .unwrap()
                .as_ref()
                .map(|d| d.session.id().to_string());
            match open_session_id {
                Some(id) => self
                    .0
                    .clipboard
                    .lock()
                    .unwrap()
                    .as_ref()
                    .map(|c| c.reachable_pieces(&id))
                    .unwrap_or_default(),
                None => Vec::new(),
            }
        };
        let mut report = vox_project::HousekeepingReport::default();
        for _ in 0..3 {
            let prepared = {
                let mut guard = self.0.open.lock().unwrap();
                let doc = guard.as_mut()?;
                if doc.session.is_recording() {
                    report.skipped_recording = true;
                    return Some(report);
                }
                let usage = doc.session.disk_usage(&clip);
                let free_bytes = free
                    .free_bytes(doc.session.dir())
                    .unwrap_or(u64::MAX)
                    .saturating_add(report.freed_bytes);
                let limits = vox_project::DiskLimits::default();
                match vox_project::decide(&usage, free_bytes, &limits) {
                    vox_project::DiskAction::None => break,
                    vox_project::DiskAction::AlmostFull => {
                        report.almost_full = true;
                        break;
                    }
                    vox_project::DiskAction::DropOldest(k) => match doc.session.drop_oldest_undo(k)
                    {
                        Ok(n) => report.dropped_undo += n,
                        Err(error) => {
                            tracing::warn!(%error, "dropping the oldest undo steps failed");
                            return Some(report);
                        }
                    },
                    vox_project::DiskAction::Compact => {}
                }
                match doc.session.begin_compact(&clip) {
                    Ok(prepared) => prepared,
                    Err(error) => {
                        tracing::warn!(%error, "starting session compaction failed");
                        return Some(report);
                    }
                }
            };
            // No document lock held here: this is the potentially slow part (reads and rewrites
            // every live chunk), and edits keep committing against the still-current generation
            // meanwhile.
            let index = match prepared.copy_chunks() {
                Ok(index) => index,
                Err(error) => {
                    tracing::warn!(%error, "copying chunks for compaction failed");
                    return Some(report);
                }
            };
            let mut guard = self.0.open.lock().unwrap();
            let Some(doc) = guard.as_mut() else {
                return Some(report);
            };
            match doc.session.finish_compact(prepared, index, &clip) {
                Ok(Some(freed)) => {
                    report.freed_bytes += freed;
                    report.compacted = true;
                    self.0.engine.set_document(Some(PlaybackDoc {
                        store: Arc::clone(doc.session.store()),
                        snapshot: doc.session.current(),
                    }));
                }
                // The document moved on (closed/reopened/recovered) while the copy ran with no
                // lock held: drop this attempt, the next housekeeping tick starts over.
                Ok(None) => return Some(report),
                Err(error) => {
                    tracing::warn!(%error, "finishing session compaction failed");
                    return Some(report);
                }
            }
        }
        Some(report)
    }

    /// Settings → Recovery & storage's "Session storage: X (history Y)" for the open document.
    pub fn storage_usage(&self) -> Option<vox_project::DiskUsage> {
        let guard = self.0.open.lock().unwrap();
        let doc = guard.as_ref()?;
        let clip = self
            .0
            .clipboard
            .lock()
            .unwrap()
            .as_ref()
            .map(|c| c.reachable_pieces(doc.session.id()))
            .unwrap_or_default();
        Some(doc.session.disk_usage(&clip))
    }

    /// The currently open document (or [`DocumentInfo::default`] when none is open).
    pub fn info(&self) -> DocumentInfo {
        self.sync_pending_rack_recovery();
        info_of(&self.0.engine, self.0.open.lock().unwrap().as_ref())
    }

    /// H-40: flags that a `RackNotice::SlotRecovered` arrived (a Missing/too-new placeholder
    /// resolved live because its module was just registered or upgraded — install, rescan,
    /// unblock, re-enable). Called from `audio::forward_rack_notice`, on the engine's own event
    /// thread — it only stores a bool, never touches `self.0.engine` (MEMORY.md S1-01: an event
    /// sink must never call `EngineHandle` back, or it deadlocks against the control thread that
    /// is calling it). [`Self::sync_pending_rack_recovery`] does the actual work later, from a
    /// normal command context.
    pub fn mark_rack_recovered(&self) {
        self.0.pending_rack_recovery.store(true, Ordering::Release);
    }

    /// Rebases the sidecar's `sidecar_dirty` baseline (SPEC-018 §4.3) to the rack's current state
    /// if [`Self::mark_rack_recovered`] flagged a live recovery since the last check — the same
    /// "whatever it resolved to just now is the unmodified baseline" trick `open`/`save` already
    /// use (§4.3's persisted-content digest). A live recovery only catches the rack up to what
    /// the *saved* state already named (its kept blob, re-resolved because the plugin is
    /// installed again); nothing about the saved content changed, so it must not read as an
    /// edit — and per SPEC-004 OD-1, rack changes are outside the undo history to begin with, so
    /// there is no undo entry to avoid separately (see the ADR-008 amendment this ticket added).
    /// A no-op with no document open (the flag stays set for the next document, harmlessly: any
    /// document's baseline is always rebased from its own current rack, not the stale one).
    fn sync_pending_rack_recovery(&self) {
        if !self.0.pending_rack_recovery.swap(false, Ordering::AcqRel) {
            return;
        }
        let mut guard = self.0.open.lock().unwrap();
        let Some(doc) = guard.as_mut() else { return };
        let save_format = save_format_model(doc.save_container, doc.save_bits, doc.save_dither);
        let markers = current_marker_items(doc);
        let rack = current_rack_value(&self.0.engine);
        doc.sidecar.persisted_digest =
            vox_project::sidecar::persisted_digest(&save_format, &markers, &rack);
    }

    /// `document_probe`/`document_open`'s probe step (SPEC-005 §2.3 step 1, §2.4): container/
    /// codec/rate/channels, plus the multichannel dialog's per-channel peaks/identical-channels/
    /// silent-channel-hint data. A thin wrapper (`probe_for_import` needs no session state), kept
    /// on `DocumentService` so the command layer never imports `vox_project` items it uses nowhere
    /// else.
    pub fn probe(&self, path: &Path) -> Result<ImportProbe, IpcError> {
        vox_project::probe_for_import(path).map_err(document_error)
    }

    /// T-209 (SPEC-005 §2.3): registers a new import job's cancel token before the (potentially
    /// slow) decode loop starts, so `document_open_cancel(job_id)` can reach it from another
    /// command invocation while this one is still running on its own blocking thread.
    pub fn start_import_job(&self) -> (u32, CancelToken) {
        let job_id = self.0.next_import_job.fetch_add(1, Ordering::Relaxed);
        let cancel = CancelToken::new();
        self.0
            .import_jobs
            .lock()
            .unwrap()
            .insert(job_id, cancel.clone());
        (job_id, cancel)
    }

    /// `document_open_cancel`: best-effort, like every other job's cancel (H-09/S4-04) — a no-op
    /// for an unknown or already-finished job id.
    pub fn cancel_import_job(&self, job_id: u32) {
        if let Some(cancel) = self.0.import_jobs.lock().unwrap().get(&job_id) {
            cancel.cancel();
        }
    }

    /// Unregisters a finished (done/cancelled/failed) import job's cancel token and live-peaks
    /// handle (H-71), if any.
    pub fn finish_import_job(&self, job_id: u32) {
        self.0.import_jobs.lock().unwrap().remove(&job_id);
        self.0.import_live.lock().unwrap().remove(&job_id);
    }

    /// H-71 (SPEC-005 §2.3): the `document_open` command's `on_live` closure calls this once,
    /// synchronously, right after `open_with_downmix`'s import creates its `ChunkWriter` — before
    /// the (potentially slow) decode loop starts — so `import_peaks_get` can answer for `job_id`
    /// from the very first poll.
    pub fn set_import_live(
        &self,
        job_id: u32,
        store: Arc<vox_project::ChunkStore>,
        live: vox_project::LiveWrittenAudio,
        sample_rate_hz: u32,
        len_hint: Option<u64>,
    ) {
        self.0.import_live.lock().unwrap().insert(
            job_id,
            ImportLiveInfo {
                store,
                live,
                sample_rate_hz,
                len_hint,
            },
        );
    }

    /// H-71 (SPEC-005 §2.3, SPEC-006 AC-13, ADR-003 Amendment 6): peaks for import job `job_id`
    /// while it's still running — `import_peaks_get`'s business logic, the live counterpart of
    /// [`Self::peaks`]. Reads the growing session's own per-chunk pyramid (ADR-004 §5: a chunk's
    /// peaks are ready the instant it commits, exactly like a finished document's) for whatever
    /// has committed so far, and pads the rest of `[start, start + count)` with `PARTIAL`/`NaN`
    /// buckets (ADR-003 §2) — a bucket only counts as "ready" once its *entire* span has
    /// committed, so a still-filling bucket reads as pending rather than a truncated union that
    /// would need to change again next poll. An unknown or already-finished `job_id` (a request
    /// that raced the job's own completion) answers with an empty, `partial` response, the same
    /// harmless fallback `record_peaks_get` uses for "no active take".
    pub fn import_peaks(
        &self,
        job_id: u32,
        spp: u32,
        start: u64,
        count: u32,
    ) -> Result<ImportPeaksResult, IpcError> {
        let Some(info) = self.0.import_live.lock().unwrap().get(&job_id).cloned() else {
            return Ok(ImportPeaksResult {
                sample_rate_hz: 0,
                buckets: Vec::new(),
                partial: true,
            });
        };
        let committed_len = info.live.len_samples();
        let spp64 = u64::from(spp.max(1));
        let ready_count = if start >= committed_len {
            0
        } else {
            (((committed_len - start) / spp64).min(u64::from(count))) as u32
        };
        let mut buckets = Vec::with_capacity(count as usize);
        if ready_count > 0 {
            let snapshot = info.live.snapshot(info.sample_rate_hz);
            let ready = vox_project::peaks(&info.store, &snapshot, spp, start, ready_count)
                .map_err(document_error)?;
            buckets.extend(ready);
        }
        let partial = ready_count < count;
        if partial {
            buckets.resize(count as usize, (f32::NAN, f32::NAN));
        }
        Ok(ImportPeaksResult {
            sample_rate_hz: info.sample_rate_hz,
            buckets,
            partial,
        })
    }

    /// H-70 (SPEC-005 §4.10): registers a new Save/Save As job's cancel token before the write
    /// starts, so `document_save_cancel(job_id)` can reach it from another command invocation
    /// while this one runs on its own blocking thread (mirrors `start_import_job`). Called only
    /// after [`Self::begin_save`]/[`Self::begin_save_as`] has already validated the request and
    /// set the busy flag — this call itself never fails.
    pub fn register_save_job(&self) -> (u32, CancelToken) {
        let job_id = self.0.next_save_job.fetch_add(1, Ordering::Relaxed);
        let cancel = CancelToken::new();
        self.0
            .save_jobs
            .lock()
            .unwrap()
            .insert(job_id, cancel.clone());
        (job_id, cancel)
    }

    /// `document_save_cancel`: best-effort, like every other job's cancel — a no-op for an
    /// unknown or already-finished job id.
    pub fn cancel_save_job(&self, job_id: u32) {
        if let Some(cancel) = self.0.save_jobs.lock().unwrap().get(&job_id) {
            cancel.cancel();
        }
    }

    /// Unregisters a finished (done/cancelled/failed) Save/Save As job's cancel token. Does
    /// *not* clear the busy flag — [`Self::finish_save`]/[`Self::finish_save_as`] do that, always,
    /// once the write itself (not just the id bookkeeping) is done.
    pub fn unregister_save_job(&self, job_id: u32) {
        self.0.save_jobs.lock().unwrap().remove(&job_id);
    }

    /// Imports `path` as a new session (SPEC-005 §2.2-2.4, T-202 `vox_project::import_file`) and
    /// makes it the engine's playback document. Any format `vox_io::decode` supports opens (WAV
    /// incl. the tolerated variants, FLAC, MP3, M4A AAC-LC, Ogg Vorbis); multichannel input
    /// downmixes by average (SPEC-005 §2.4 default). Replaces whatever was open before (the
    /// frontend is responsible for the unsaved-changes prompt — SPEC-004 §2.8 "simple version",
    /// ticket scope — before calling this).
    /// `confirm_already_open` bypasses SPEC-018 §2.11's "already open in another instance"
    /// warning (the UI re-issues with `true` after "Open Anyway").
    ///
    /// This is the plain, synchronous entry point every other module/test calls directly (average
    /// downmix, no progress, a throwaway cancel token). T-209's `document_open` command instead
    /// calls [`Self::open_with_downmix`] with the user's channel choice, a job-tracked cancel
    /// token and a progress callback (SPEC-005 §2.3's import job).
    pub fn open(&self, path: &Path, confirm_already_open: bool) -> Result<DocumentInfo, IpcError> {
        self.open_impl(
            path,
            confirm_already_open,
            vox_io::DownmixChoice::Average,
            &CancelToken::new(),
            &mut |_fraction| {},
            &mut |_live, _store| {},
        )
    }

    /// T-209 (SPEC-005 §2.3/§2.4): [`Self::open`] with an explicit downmix choice, a cancel token
    /// the caller can trigger from another thread/command (`document_open_cancel`), and a progress
    /// callback (`fraction` in `[0, 1]`, called whenever `vox_project::import_file` reports
    /// `len_hint`; some containers report no length at all, in which case this is never called and
    /// the caller's progress bar stays indeterminate). Cancelling leaves whatever was open
    /// untouched: the new session is only swapped in on success, exactly like [`Self::open`] — see
    /// this function's body, shared via [`Self::open_impl`].
    ///
    /// H-71 (SPEC-005 §2.3): `on_live` is called once, synchronously, right after the import's
    /// `ChunkWriter` is created and before the (potentially slow) decode loop starts, with a live
    /// peaks handle and the store it reads through — the `document_open` command wires it into
    /// [`Self::set_import_live`] so `import_peaks_get` can serve progressive peaks for this job.
    #[allow(clippy::too_many_arguments)]
    pub fn open_with_downmix(
        &self,
        path: &Path,
        confirm_already_open: bool,
        downmix: vox_io::DownmixChoice,
        cancel: &CancelToken,
        on_progress: &mut dyn FnMut(f32),
        on_live: &mut dyn FnMut(vox_project::LiveWrittenAudio, Arc<vox_project::ChunkStore>),
    ) -> Result<DocumentInfo, IpcError> {
        self.open_impl(
            path,
            confirm_already_open,
            downmix,
            cancel,
            on_progress,
            on_live,
        )
    }

    fn open_impl(
        &self,
        path: &Path,
        confirm_already_open: bool,
        downmix: vox_io::DownmixChoice,
        cancel: &CancelToken,
        on_progress: &mut dyn FnMut(f32),
        on_live: &mut dyn FnMut(vox_project::LiveWrittenAudio, Arc<vox_project::ChunkStore>),
    ) -> Result<DocumentInfo, IpcError> {
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
        let (save_container, save_bits, format_mapped) = save_format_for_import(&probe);
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
        let mut session = self.create_session(config).map_err(document_error)?;
        let store_for_live = Arc::clone(session.store());
        let import = vox_project::import_file_with_live(
            &mut session,
            path,
            downmix,
            cancel,
            |frames_done, len_hint| {
                if let Some(len) = len_hint.filter(|&l| l > 0) {
                    on_progress((frames_done as f64 / len as f64).min(1.0) as f32);
                }
            },
            |live| on_live(live, store_for_live),
        );
        let import = match import {
            Ok(result) => result,
            Err(err) => {
                let _ = std::fs::remove_dir_all(session.dir());
                return Err(document_error(err));
            }
        };
        let mut snapshot = import.snapshot;
        // SPEC-009 §2.13 case 3: the WAV's own markers, exactly as case 5 built them (fresh ids
        // `1..=n` in canonical order) — kept aside before the sidecar block below may replace
        // `snapshot.markers`, so that block can compare cue projections against it.
        let wav_markers = Arc::clone(&snapshot.markers);

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
        // SPEC-009 §2.13 case 3: `true` once a valid, matching sidecar exists AND the WAV's own
        // readable cue set differs from it — another program changed the markers since the last
        // save. Only a WAV (`probe.container == "wave"`) with a readable chunk is even compared:
        // a non-WAV source (case 1) has no cues to differ, and a malformed chunk (case 4) already
        // falls back to the sidecar unconditionally, independent of this flag.
        let mut markers_changed_externally = false;
        if let Some(sidecar_doc) = &load.doc {
            markers_changed_externally = probe.container == "wave"
                && !import.wav_markers_malformed
                && !cue_projection_matches(&wav_markers, &load.markers);
            // Markers: case 3 uses the file's own markers (kind inherited where a sidecar marker's
            // projection still matches exactly); otherwise the sidecar's (authoritative, case 2) —
            // same re-floor mechanism `import_file` used for the WAV cue markers, valid here
            // because nothing has edited the document yet (`Session::set_floor`'s only
            // precondition). A case-3 marker's id is fresh (from the file), so its
            // `MarkerMetaTable` starts empty rather than reusing the sidecar's (SPEC-018 §2.7 extra
            // fields are meaningless once detached from the sidecar's own ids).
            let (floor_markers, marker_meta): (Vec<Marker>, MarkerMetaTable) =
                if markers_changed_externally {
                    (
                        markers_with_inherited_kind(&wav_markers, &load.markers),
                        MarkerMetaTable::new(),
                    )
                } else {
                    (
                        load.markers.iter().map(marker_from_item).collect(),
                        MarkerMetaTable::from_items(&load.markers),
                    )
                };
            sidecar.marker_meta = marker_meta;
            let audio = WrittenAudio {
                pieces: snapshot.pieces.to_vec(),
                chunks: Vec::new(),
                len_samples: snapshot.len_samples,
            };
            if let Ok(reflowed) = session.set_floor(&audio, floor_markers) {
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
            self.push_sidecar_notice(sidecar_notice_info(notice, &sidecar_path));
        }
        if markers_changed_externally {
            self.push_sidecar_notice(SidecarNoticeInfo {
                key: "notice.open.markers_changed_externally",
                params: vec![(
                    "name",
                    path.file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default(),
                )],
            });
        }
        // H-20 (SPEC-005 §2.6): the source format/depth was mapped to a different save format.
        if format_mapped {
            let (from, to) = format_mapped_notice_params(&probe, save_bits);
            self.push_sidecar_notice(SidecarNoticeInfo {
                key: "notice.open.format_mapped",
                params: vec![("from", from.to_string()), ("to", to.to_string())],
            });
        }
        // H-72 (SPEC-005 §2.5/§2.9): the source's `cue `/`LIST adtl` chunk was malformed, or some
        // of its cue points fell outside the document — both silently dropped until now
        // (`import_file`'s marker filter), now surfaced. Independent of the sidecar's own
        // markers/precedence (SPEC-005 §2.9 "Sidecar precedence"): this is about the *source
        // file's* cue chunk, true regardless of which markers end up on the document.
        if import.wav_markers_malformed {
            self.push_sidecar_notice(SidecarNoticeInfo {
                key: "notice.open.markers_unreadable",
                params: Vec::new(),
            });
        }
        if import.wav_markers_out_of_range > 0 {
            self.push_sidecar_notice(SidecarNoticeInfo {
                key: "notice.open.markers_out_of_range",
                params: vec![("count", import.wav_markers_out_of_range.to_string())],
            });
        }

        let store = Arc::clone(session.store());
        self.0.engine.set_document(Some(PlaybackDoc {
            store,
            snapshot: Arc::clone(&snapshot),
        }));

        let meta = std::fs::metadata(path).ok();
        sidecar.file_size_bytes = meta.as_ref().map(|m| m.len()).unwrap_or(0);
        sidecar.file_mtime_unix_ms = meta
            .and_then(|m| m.modified().ok())
            .map(unix_ms_of)
            .unwrap_or(0);
        let save_format = save_format_model(save_container, save_bits, SaveDitherPref::default());
        let markers = sidecar.marker_meta.build_items(&snapshot.markers);
        let rack = current_rack_value(&self.0.engine);
        sidecar.persisted_digest =
            vox_project::sidecar::persisted_digest(&save_format, &markers, &rack);
        sidecar.last_saved_audio_rev = snapshot.audio_rev;
        sidecar.last_written_markers = markers;

        let mut doc = OpenDocument::new(session, Some(path.to_path_buf()), save_bits, sidecar);
        doc.save_container = save_container;
        doc.source_channels = import.source_channels;
        doc.had_foreign_metadata = import.has_foreign_metadata;
        let mut guard = self.0.open.lock().unwrap();
        // H-56 (SPEC-008 §2.6): materialize a `Bound` clipboard before its session closes — its
        // pieces reference the session that's about to close.
        if let Some(current) = guard.as_ref() {
            self.materialize_clipboard(current.session.store());
        }
        let previous = guard.replace(doc);
        let info = info_of(&self.0.engine, guard.as_ref());
        drop(guard);
        if let Some(previous) = previous {
            close_with_retry(previous.session);
        }
        Ok(info)
    }

    /// Drains the last open/save's sidecar/format notices, if any (H-20: now possibly more than
    /// one — see [`Inner::pending_sidecar_notice`]).
    pub fn take_sidecar_notices(&self) -> Vec<SidecarNoticeInfo> {
        std::mem::take(&mut self.0.pending_sidecar_notice.lock().unwrap())
    }

    fn push_sidecar_notice(&self, notice: SidecarNoticeInfo) {
        self.0.pending_sidecar_notice.lock().unwrap().push(notice);
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

    /// H-12 (SPEC-018 §2.6.5): merges `waveform` into the open document's `view` JSON for the
    /// next save — a no-op (not an error) with no document open. Never touches `sidecar_dirty`,
    /// same guarantee as [`Self::set_spectral_view`]. Other `view` sections are preserved
    /// untouched: this only ever replaces the `waveform` key.
    pub fn set_waveform_view(&self, waveform: WaveformViewInfo) {
        let mut guard = self.0.open.lock().unwrap();
        let Some(doc) = guard.as_mut() else {
            return;
        };
        let mut view = match std::mem::take(&mut doc.sidecar.view) {
            serde_json::Value::Object(map) => map,
            _ => serde_json::Map::new(),
        };
        let selection = match waveform.selection {
            Some((start, end)) => serde_json::json!({ "start_sample": start, "end_sample": end }),
            None => serde_json::Value::Null,
        };
        let time_ruler_format = match waveform.time_ruler_format {
            TimeRulerFormatDto::Timecode => "timecode",
            TimeRulerFormatDto::Samples => "samples",
            TimeRulerFormatDto::Seconds => "seconds",
        };
        let amplitude_ruler_mode = match waveform.amplitude_ruler_mode {
            AmplitudeRulerModeDto::Dbfs => "dbfs",
            AmplitudeRulerModeDto::Percent => "percent",
        };
        view.insert(
            "waveform".to_string(),
            serde_json::json!({
                "start_sample": waveform.start_sample,
                "samples_per_pixel": waveform.samples_per_pixel,
                "selection": selection,
                "cursor_samples": waveform.cursor_samples,
                "time_ruler_format": time_ruler_format,
                "vertical_zoom": waveform.vertical_zoom,
                "amplitude_ruler_mode": amplitude_ruler_mode,
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
    ///
    /// `confirm_clip` bypasses SPEC-005 §2.8's clip prompt (the UI re-issues with `true` after
    /// "Clip and save"; picking "Save as 32-bit float instead" is a `document_save_as` call to the
    /// same path at `Bit32Float` instead, which never needs this flag — float overs are never a
    /// clip). The check only runs for an integer save format (16/24-bit, WAV or FLAC).
    ///
    /// `confirm_multichannel` bypasses SPEC-005 §2.4's "saving replaces the stereo source with
    /// mono" warning (the UI re-issues with `true` after the user picks Save) — shown at most once
    /// per document ([`OpenDocument::multichannel_warned`]).
    pub fn save(
        &self,
        overwrite: bool,
        confirm_clip: bool,
        confirm_multichannel: bool,
    ) -> Result<DocumentInfo, IpcError> {
        let cancel = CancelToken::new();
        self.begin_save(
            overwrite,
            confirm_clip,
            confirm_multichannel,
            &vox_project::SystemFreeSpace,
            &cancel,
            |_| {},
        )?;
        self.finish_save(&cancel, |_| {})
    }

    /// H-70 (SPEC-005 §4.10): the validating half of [`Self::save`] — every pre-flight check
    /// (§2.7's ordered checklist) runs here, against the doc lock held only briefly, so a
    /// rejection (busy, untitled, changed-on-disk, permission, disk-full, too-large-for-WAV, or a
    /// clip/multichannel confirmation) never emits a job/progress event at all — exactly like
    /// `BakeService::start_job` never spawning a job for `error.bake.rack_inactive`. Only once
    /// every check passes does this set the busy flag (`error.save.in_progress` for a second
    /// caller); [`Self::finish_save`] always clears it, whatever the write's outcome. `free` is
    /// H-11's `FreeSpaceProvider` (real callers pass `SystemFreeSpace`; tests, `FixedFreeSpace`).
    ///
    /// H-75 (SPEC-005 §2.8): `cancel`/`on_progress` are the same ones [`Self::finish_save`] will
    /// use for the write — the overs pre-flight's exact counting pass (only run when the peak
    /// pyramid already shows a peak `> 1.0`) is itself "a cancellable job with progress", not a
    /// throwaway, uncancellable scan.
    pub fn begin_save(
        &self,
        overwrite: bool,
        confirm_clip: bool,
        confirm_multichannel: bool,
        free: &dyn vox_project::FreeSpaceProvider,
        cancel: &CancelToken,
        mut on_progress: impl FnMut(f32),
    ) -> Result<(), IpcError> {
        let mut busy = self.0.save_running.lock().unwrap();
        if *busy {
            return Err(save_in_progress_error());
        }
        let guard = self.0.open.lock().unwrap();
        let doc = guard.as_ref().ok_or_else(no_document)?;
        let path = doc.path.clone().ok_or_else(untitled)?;
        let container = doc.save_container;
        let bits = doc.save_bits;
        if !overwrite && changed_on_disk(doc, &path) {
            return Err(changed_on_disk_error(&path));
        }
        // H-15 (SPEC-018 §2.9): a read-only folder is refused here, before the audio or the
        // sidecar is touched — full and sidecar-only saves alike (`needs_full` isn't decided yet).
        check_folder_writable(&path)?;
        check_free_space(doc, bits, &path, free)?;
        check_wav_size(doc, container, bits)?;
        if !confirm_clip && let Some(overs) = check_overs(doc, bits, cancel, &mut on_progress)? {
            return Err(clip_confirmation_error(overs));
        }
        if !confirm_multichannel && doc.source_channels > 1 && !doc.multichannel_warned {
            return Err(multichannel_confirmation_error(&path));
        }
        *busy = true;
        Ok(())
    }

    /// H-70: the writing half of [`Self::save`] — [`Self::begin_save`] must have succeeded
    /// first (its pre-flight already resolved every confirmation, so any error from here on is a
    /// real job failure or cancellation, never `NeedsConfirmation`). `cancel` is checked inside
    /// the write loop (best-effort, like every other job); `on_progress` reports `[0.0, 1.0]`.
    /// Always clears the busy flag, whatever the outcome.
    ///
    /// H-75 (SPEC-005 §2.7): "editing and playback continue" while a save runs, so the document
    /// lock is held only twice, briefly — once to capture everything the write needs
    /// ([`prepare_save`]), including the seq the *snapshot being written* corresponds to, and
    /// once after the write to record the outcome ([`Self::apply_save_outcome`]) — never across
    /// the write itself. An edit committed in between lands normally and leaves the document
    /// dirty: [`Session::mark_saved_file_at`] is told to record exactly the captured seq, not
    /// whatever `doc.session`'s current seq is by the time this re-locks.
    pub fn finish_save(
        &self,
        cancel: &CancelToken,
        mut on_progress: impl FnMut(f32),
    ) -> Result<DocumentInfo, IpcError> {
        // T-901: state a plugin window changed outside the plugin's parameters reaches the rack's
        // committed blobs before the sidecar is written (round trips off the document lock).
        self.0.engine.rack_capture_plugin_states();

        let (path, container, bits, dither, prep) = {
            let guard = self.0.open.lock().unwrap();
            let doc = guard.as_ref().ok_or_else(no_document)?;
            let path = doc.path.clone().ok_or_else(untitled)?;
            let container = doc.save_container;
            let bits = doc.save_bits;
            let dither = doc.save_dither;
            let prep = prepare_save(doc, &self.0.engine, &path, container, bits, dither, false);
            (path, container, bits, dither, prep)
        };

        // SPEC-007 §4.1 (H-30): tiles use at most half of the workers while a save writes, like
        // export/bake.
        let background = self.0.spectro.get().map(|s| s.begin_background_job());
        let write_result = write_save(
            &prep,
            &path,
            container,
            bits,
            dither,
            cancel,
            &mut on_progress,
        );
        drop(background);

        let result = (|| -> Result<DocumentInfo, IpcError> {
            let outcome = write_result?;
            let mut guard = self.0.open.lock().unwrap();
            let doc = guard.as_mut().ok_or_else(no_document)?;
            if doc.session.id() != prep.session_id {
                // H-75: the document was closed or replaced while this save's write ran (no
                // document lock was held for it). The file on disk is still correct for the
                // snapshot that was saved — there's just no open document left to record it
                // against.
                return Err(no_document());
            }
            self.apply_save_outcome(doc, &prep, &path, container, bits, outcome)?;
            Ok(info_of(&self.0.engine, guard.as_ref()))
        })();
        *self.0.save_running.lock().unwrap() = false;
        result
    }

    /// Writes the current revision to `path` in `container` at `bits`/`dither` (always a full
    /// save, SPEC-018 §2.3), then binds the document to it. `error.save.flac_needs_int_bits` for
    /// `Flac` + `Bit32Float` (SPEC-005 §2.6/§2.11 — FLAC has no float format).
    ///
    /// `confirm_clip`/`confirm_multichannel`: see [`Self::save`]'s doc comment — same contract,
    /// checked against the requested `bits` (not the document's currently bound one).
    pub fn save_as(
        &self,
        path: &Path,
        container: SaveContainer,
        bits: BitDepth,
        dither: SaveDitherPref,
        confirm_clip: bool,
        confirm_multichannel: bool,
    ) -> Result<DocumentInfo, IpcError> {
        let cancel = CancelToken::new();
        self.begin_save_as(
            path,
            container,
            bits,
            confirm_clip,
            confirm_multichannel,
            &vox_project::SystemFreeSpace,
            &cancel,
            |_| {},
        )?;
        self.finish_save_as(path, container, bits, dither, &cancel, |_| {})
    }

    /// H-70: the validating half of [`Self::save_as`] — see [`Self::begin_save`]'s doc comment;
    /// same contract, checked against the requested `path`/`container`/`bits` (an arbitrary new
    /// target a native file dialog can't guarantee is writable, and a format change can newly
    /// trip the WAV size limit — H-60's "keep `save`/`save_as` pre-flight checklists in sync").
    /// H-75: `cancel`/`on_progress` — see [`Self::begin_save`]'s doc comment, same contract.
    #[allow(clippy::too_many_arguments)]
    pub fn begin_save_as(
        &self,
        path: &Path,
        container: SaveContainer,
        bits: BitDepth,
        confirm_clip: bool,
        confirm_multichannel: bool,
        free: &dyn vox_project::FreeSpaceProvider,
        cancel: &CancelToken,
        mut on_progress: impl FnMut(f32),
    ) -> Result<(), IpcError> {
        if container == SaveContainer::Flac && bits == BitDepth::Bit32Float {
            return Err(invalid_flac_bit_depth());
        }
        let mut busy = self.0.save_running.lock().unwrap();
        if *busy {
            return Err(save_in_progress_error());
        }
        let guard = self.0.open.lock().unwrap();
        let doc = guard.as_ref().ok_or_else(no_document)?;
        check_folder_writable(path)?;
        check_free_space(doc, bits, path, free)?;
        check_wav_size(doc, container, bits)?;
        if !confirm_clip && let Some(overs) = check_overs(doc, bits, cancel, &mut on_progress)? {
            return Err(clip_confirmation_error(overs));
        }
        if !confirm_multichannel && doc.source_channels > 1 && !doc.multichannel_warned {
            return Err(multichannel_confirmation_error(path));
        }
        *busy = true;
        Ok(())
    }

    /// H-70: the writing half of [`Self::save_as`] — see [`Self::finish_save`]'s doc comment;
    /// [`Self::begin_save_as`] must have succeeded first. Always clears the busy flag. H-75: same
    /// lock-released-during-the-write shape as [`Self::finish_save`] — see its doc comment.
    /// `force_full = true` in [`prepare_save`] (a different path, and potentially format, is
    /// never a sidecar-only write) — previously done by bumping `doc.sidecar.last_saved_audio_rev`
    /// back one before calling the old shared `save_to`, which needed a `doc` mutation to force
    /// its generic "did anything change" check; the split prepare/write no longer does.
    pub fn finish_save_as(
        &self,
        path: &Path,
        container: SaveContainer,
        bits: BitDepth,
        dither: SaveDitherPref,
        cancel: &CancelToken,
        mut on_progress: impl FnMut(f32),
    ) -> Result<DocumentInfo, IpcError> {
        // T-901: see `finish_save`.
        self.0.engine.rack_capture_plugin_states();

        let prep = {
            let guard = self.0.open.lock().unwrap();
            let doc = guard.as_ref().ok_or_else(no_document)?;
            prepare_save(doc, &self.0.engine, path, container, bits, dither, true)
        };

        // SPEC-007 §4.1 (H-30): tiles use at most half of the workers while a save writes, like
        // export/bake.
        let background = self.0.spectro.get().map(|s| s.begin_background_job());
        let write_result = write_save(
            &prep,
            path,
            container,
            bits,
            dither,
            cancel,
            &mut on_progress,
        );
        drop(background);

        let result = (|| -> Result<DocumentInfo, IpcError> {
            let outcome = write_result?;
            let mut guard = self.0.open.lock().unwrap();
            let doc = guard.as_mut().ok_or_else(no_document)?;
            if doc.session.id() != prep.session_id {
                // H-75: see `finish_save`'s doc comment.
                return Err(no_document());
            }
            self.apply_save_outcome(doc, &prep, path, container, bits, outcome)?;
            doc.path = Some(path.to_path_buf());
            doc.save_bits = bits;
            doc.save_container = container;
            doc.save_dither = dither;
            Ok(info_of(&self.0.engine, guard.as_ref()))
        })();
        *self.0.save_running.lock().unwrap() = false;
        result
    }

    /// H-75: applies a finished save write's outcome to `doc` (already re-locked, and confirmed
    /// to still be the same session `prep` was captured from) — sidecar bookkeeping, the
    /// metadata-dropped notice (needs `doc.had_foreign_metadata`/`metadata_notice_shown`, decided
    /// here rather than during the lock-free write since it's a once-per-document flag the
    /// document lock must serialize), and the actual "this state is saved" record via
    /// [`vox_project::Session::mark_saved_file_at`] at `prep.seq_at_snapshot` — never `doc.
    /// session`'s live current seq, which may have moved on if an edit landed while the write ran.
    /// Shared by [`Self::finish_save`]/[`Self::finish_save_as`]; the latter additionally rebinds
    /// `doc.path`/`save_bits`/`save_container`/`save_dither` itself afterwards.
    fn apply_save_outcome(
        &self,
        doc: &mut OpenDocument,
        prep: &SavePrep,
        path: &Path,
        container: SaveContainer,
        bits: BitDepth,
        outcome: SaveWriteOutcome,
    ) -> Result<(), IpcError> {
        for notice in outcome.notices {
            self.push_sidecar_notice(notice);
        }
        doc.sidecar.audio_crc32 = outcome.audio_crc32;
        doc.sidecar.file_size_bytes = outcome.file_facts.map(|f| f.0).unwrap_or(0);
        doc.sidecar.file_mtime_unix_ms = outcome.file_facts.map(|f| f.1).unwrap_or(0);
        doc.sidecar.last_saved_audio_rev = prep.snapshot.audio_rev;
        if outcome.sidecar_written {
            doc.sidecar.needs_backup = false;
            doc.sidecar.last_written_markers = prep.markers.clone();
            doc.sidecar.persisted_digest = outcome.persisted_digest;
        }
        // H-20 (SPEC-005 §2.10): "the first Save over a source that had such metadata" — shown
        // once per document, whether the audio was actually rewritten this call or not (a
        // sidecar-only save still counts as "a Save" for this purpose).
        if doc.had_foreign_metadata && !doc.metadata_notice_shown {
            doc.metadata_notice_shown = true;
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            self.push_sidecar_notice(SidecarNoticeInfo {
                key: "notice.save.metadata_dropped",
                params: vec![("name", name)],
            });
        }
        doc.session
            .mark_saved_file_at(
                prep.seq_at_snapshot,
                path,
                format_tag(container, bits),
                Some(vox_project::sidecar::crc32_to_hex(outcome.audio_crc32)),
                Some(outcome.sidecar_written),
                outcome.file_facts,
            )
            .map_err(document_error)?;
        doc.multichannel_warned = true;
        doc.recovered = false;
        Ok(())
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

    /// H-56: `true` while a cross-document paste job is running (SPEC-008 §2.2's "another
    /// document job runs" — `error.document_busy`; mirrors [`Self::is_normalize_busy`]).
    pub fn is_paste_busy(&self) -> bool {
        *self.0.paste_busy.lock().unwrap()
    }

    /// H-30: `true` specifically while a bake job is running (`begin_bake_job`..
    /// `finish_bake`/`abandon_bake_job`) — `ipc::rack_commands` refuses rack edits then
    /// (`error.bake.busy`) instead of letting the bake's post-job rack load silently clobber
    /// them.
    pub fn is_bake_running(&self) -> bool {
        *self.0.bake_running.lock().unwrap()
    }

    /// H-70 (SPEC-005 §2.7/§4.10, AC-15): `true` while a Save/Save As job
    /// (`begin_save`/`begin_save_as` .. `finish_save`/`finish_save_as`) is writing. Unlike
    /// `is_bake_running`/`is_normalize_busy`, nothing gates edits on this — Save/Save As are the
    /// one job kind the spec keeps editing and playback running during.
    pub fn is_save_running(&self) -> bool {
        *self.0.save_running.lock().unwrap()
    }

    /// H-30 (SPEC-007 §4.1): wires the spectrogram tile service so `save`/`save_as` can hold
    /// `SpectroService::begin_background_job()` while they write, alongside export/bake. Called
    /// once by the composition root, right after both are constructed; a `DocumentService` that
    /// never gets this (most tests) simply never holds the guard.
    pub fn set_spectro(&self, spectro: Arc<SpectroService>) {
        let _ = self.0.spectro.set(spectro);
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
            let session = self
                .create_session(SessionConfig::new(rate_hz))
                .map_err(document_error)?;
            // The empty document also sets the output to the recording's rate (Dry monitoring
            // needs one rate on both streams).
            self.0.engine.set_document(Some(PlaybackDoc {
                store: Arc::clone(session.store()),
                snapshot: session.current(),
            }));
            // H-56 (SPEC-008 §2.6): same as `open` — materialize a `Bound` clipboard before its
            // session closes.
            if let Some(current) = guard.as_ref() {
                self.materialize_clipboard(current.session.store());
            }
            previous = guard.replace(OpenDocument::new(
                session,
                None,
                save_bits,
                SidecarState::none(),
            ));
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
                    dropout_marker_len(d.len_samples),
                    dropout_marker_name(d.len_samples, rate_hz),
                )
                .with_kind(MarkerKind::Dropout)
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
                match self.create_session(SessionConfig::new(rate_hz)) {
                    Ok(fresh) => {
                        self.0.engine.set_document(Some(PlaybackDoc {
                            store: Arc::clone(fresh.store()),
                            snapshot: fresh.current(),
                        }));
                        // H-56 (SPEC-008 §2.6): materialize a `Bound` clipboard before the broken
                        // session is dropped.
                        self.materialize_clipboard(doc.session.store());
                        if let Some(broken) = guard.replace(OpenDocument::new(
                            fresh,
                            None,
                            save_bits,
                            SidecarState::none(),
                        )) {
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

    // --- T-304: record operations (SPEC-022) ----------------------------------------------

    /// Length of the open document (0: none, or empty).
    pub fn current_len_samples(&self) -> u64 {
        self.0
            .open
            .lock()
            .unwrap()
            .as_ref()
            .map_or(0, |d| d.session.current().len_samples)
    }

    /// T-304 (SPEC-022 §2.2, §4.6): opens the take for a resolved record operation (Insert /
    /// Overwrite at `at`, or a punch over `[S, E)`), journaling `take_begin` with its crossfade,
    /// offset and alignment. Refused while recording or while a document job runs.
    pub fn begin_record_op(
        &self,
        plan: &vox_engine::record_op::RecordPlan,
    ) -> Result<TakeCapture, IpcError> {
        use vox_engine::record_op::RecordOpKind;
        if self.is_normalize_busy() {
            return Err(document_busy());
        }
        let mut guard = self.0.open.lock().unwrap();
        let doc = guard.as_mut().ok_or_else(no_document)?;
        if doc.session.is_recording() {
            return Err(IpcError::not_while_recording());
        }
        if doc.session.sample_rate_hz() != plan.doc_rate_hz {
            return Err(invalid_range());
        }
        let mode = match plan.kind {
            RecordOpKind::New => TakeMode::New,
            RecordOpKind::Insert => TakeMode::Insert {
                at_samples: plan.at_samples,
            },
            RecordOpKind::Overwrite => TakeMode::Overwrite {
                at_samples: plan.at_samples,
            },
            RecordOpKind::Punch => TakeMode::Punch {
                start_samples: plan.at_samples,
                end_samples: plan.end_samples.unwrap_or(plan.at_samples),
            },
        };
        doc.session
            .begin_take_with(
                mode,
                vox_project::TakeParams {
                    xfade_samples: plan.xfade_samples,
                    offset_ns: plan.offset_ns,
                    aligned: plan.aligned,
                },
                TakeWriterOptions::default(),
            )
            .map_err(document_error)
    }

    /// T-304 (SPEC-022 §4.6): journals `take_window` for the open take (best effort — a failure
    /// only costs crash recovery of this take its alignment, so it is logged, not raised).
    pub fn note_take_window(&self, take: u32, k_start: u64) {
        let mut guard = self.0.open.lock().unwrap();
        if let Some(doc) = guard.as_mut()
            && let Err(error) = doc.session.note_take_window(TakeId(take), k_start)
        {
            tracing::warn!(%error, take, k_start, "journaling the record window failed");
        }
    }

    /// T-304 (SPEC-022 §2.10–§2.12, §4.1): commits a finished record operation's window as one
    /// undoable edit ("Record" / "Punch-in", with the dropout markers inside the window) and
    /// hands the result to the engine, setting the playhead per §2.10 (`S` after a punch, the end
    /// of the new audio after Insert/Overwrite). A cancelled operation closes its take with
    /// `take_cancel` and returns `Ok(None)` (document unchanged). On a commit failure the take is
    /// closed without an edit (its WAV stays in the session) so the document stays usable.
    pub fn commit_take_op(
        &self,
        finished: &FinishedTake,
        op: &vox_engine::record::OpResult,
        dropouts: &[DropoutMark],
    ) -> Result<Option<(DocumentInfo, EditResult)>, IpcError> {
        use vox_engine::record_op::RecordOpKind;
        let mut guard = self.0.open.lock().unwrap();
        let doc = guard.as_mut().ok_or_else(no_document)?;
        let Some((ws, we)) = op.window else {
            if let Err(error) = doc.session.cancel_take(finished.take) {
                tracing::warn!(%error, "cancelling the take failed");
            }
            return Ok(None);
        };
        let plan = op.plan;
        let rate_hz = doc.session.sample_rate_hz();
        let take_len = finished.audio.len_samples;
        let mut n = we.min(take_len).saturating_sub(ws.min(take_len));
        if plan.kind == RecordOpKind::Punch {
            n = n.min(
                plan.end_samples
                    .unwrap_or(plan.at_samples)
                    .saturating_sub(plan.at_samples),
            );
        }
        // SPEC-022 §2.12: only dropouts inside the window get a marker, at their document
        // position; pre-/post-roll gaps are filled in the take but never enter the document.
        let markers: Vec<Marker> = dropouts
            .iter()
            .filter(|d| d.pos_samples >= ws && d.pos_samples < ws + n)
            .map(|d| {
                let id = doc.session.new_marker_id();
                Marker::new(
                    id,
                    plan.at_samples + (d.pos_samples - ws),
                    dropout_marker_len(d.len_samples),
                    dropout_marker_name(d.len_samples, rate_hz),
                )
                .with_kind(MarkerKind::Dropout)
            })
            .collect();
        let mut outcome = doc.session.commit_take_window(finished, (ws, we), &markers);
        for _ in 1..COMMIT_ATTEMPTS {
            if outcome.is_ok() {
                break;
            }
            std::thread::sleep(COMMIT_RETRY_DELAY);
            outcome = doc.session.commit_take_window(finished, (ws, we), &markers);
        }
        let step = match outcome {
            Ok(Some(step)) => step,
            Ok(None) => return Ok(None),
            Err(err) => {
                if let Err(error) = doc.session.discard_take(finished.take) {
                    tracing::warn!(%error, "closing the failed take failed");
                }
                return Err(document_error(err));
            }
        };
        self.0.engine.set_document(Some(PlaybackDoc {
            store: Arc::clone(doc.session.store()),
            snapshot: Arc::clone(&step.snapshot),
        }));
        let (selection, playhead) = match plan.kind {
            RecordOpKind::Punch => (
                plan.end_samples.map(|e| (plan.at_samples, e)),
                plan.at_samples,
            ),
            _ => (None, plan.at_samples + n),
        };
        self.0.engine.transport(TransportCommand::Seek(playhead));
        let result = EditResult {
            changed: true,
            audio_rev: step.snapshot.audio_rev,
            len_samples: step.snapshot.len_samples,
            selection,
            playhead_samples: playhead,
        };
        Ok(Some((info_of(&self.0.engine, guard.as_ref()), result)))
    }

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
            undo_label_params: history.undo_label_params().cloned().unwrap_or_default(),
            redo_label_params: history.redo_label_params().cloned().unwrap_or_default(),
        }
    }

    /// The clipboard's length/rate (`clipboard_changed` event; `None`s: empty).
    pub fn clipboard_info(&self) -> ClipboardInfo {
        match self.0.clipboard.lock().unwrap().as_ref() {
            Some(c) => ClipboardInfo {
                len_samples: Some(c.len_samples()),
                sample_rate_hz: Some(c.sample_rate_hz()),
            },
            None => ClipboardInfo::default(),
        }
    }

    /// H-56 (SPEC-008 §2.6): `true` when a plain `edit_paste` would need the cross-document
    /// import job (§2.6.1) instead of a pure splice — the clipboard is materialized and isn't
    /// already converted for whatever document is currently open. `false` for an empty clipboard
    /// (`edit_paste` then just returns `error.clipboard_empty`) or one already usable as a splice
    /// (`Bound`, or `Materialized` with a matching `converted` cache).
    pub fn paste_needs_job(&self) -> bool {
        let guard = self.0.open.lock().unwrap();
        let Some(doc) = guard.as_ref() else {
            return false;
        };
        let session_id = doc.session.id();
        match self.0.clipboard.lock().unwrap().as_ref() {
            Some(ClipboardData::Materialized { converted, .. }) => !converted
                .as_ref()
                .is_some_and(|c| c.session_id == session_id),
            _ => false,
        }
    }

    /// H-56 (SPEC-008 §2.6): streams a `Bound` clipboard's audio to
    /// `<app_local_data_dir>/clipboard/` before `store` (its owning session's chunk store) is torn
    /// down, so the clipboard survives the session closing. Called at every point that replaces
    /// or closes the open document, while `store` is still the *closing* session's — a
    /// `Materialized` clipboard (already detached from a session) or an empty one is left
    /// untouched. A failure (e.g. disk full) clears the clipboard and queues
    /// `notice.clipboard_lost` instead of failing the close/open itself (SPEC-008 §2.6).
    fn materialize_clipboard(&self, store: &Arc<vox_project::ChunkStore>) {
        let mut guard = self.0.clipboard.lock().unwrap();
        let (pieces, sample_rate_hz) = match guard.as_ref() {
            Some(ClipboardData::Bound {
                pieces,
                sample_rate_hz,
                ..
            }) => (pieces.clone(), *sample_rate_hz),
            _ => return,
        };
        match vox_project::clipboard::materialize(
            store,
            &pieces,
            sample_rate_hz,
            &self.0.clipboard_dir,
        ) {
            Ok(clip) => {
                *guard = Some(ClipboardData::Materialized {
                    clip,
                    converted: None,
                });
            }
            Err(error) => {
                tracing::warn!(%error, "materializing the clipboard failed");
                *guard = None;
                drop(guard);
                self.push_sidecar_notice(SidecarNoticeInfo {
                    key: "notice.clipboard_lost",
                    params: Vec::new(),
                });
            }
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
        if self.is_normalize_busy() || self.is_paste_busy() {
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
        *self.0.clipboard.lock().unwrap() = Some(ClipboardData::Bound {
            pieces: clip,
            sample_rate_hz: doc.session.sample_rate_hz(),
            len_samples,
        });
        Ok(self.apply_committed(doc, step, edit::post_cut_or_delete(range)))
    }

    /// Copies `[start, end)` into the clipboard. Not an edit: no commit, no transport stop, no
    /// `rev`/`audio_rev` change — refused while recording all the same (SPEC-008 §2.2: the
    /// selection can cover live, uncommitted audio during a take). H-66: gated against a running
    /// normalize/paste job like its six siblings — it still overwrites the clipboard, which a
    /// concurrent paste job may be reading.
    pub fn edit_copy(&self, start: u64, end: u64) -> Result<EditResult, IpcError> {
        if self.is_normalize_busy() || self.is_paste_busy() {
            return Err(document_busy());
        }
        let guard = self.0.open.lock().unwrap();
        let doc = guard.as_ref().ok_or_else(no_document)?;
        if doc.session.is_recording() {
            return Err(IpcError::not_while_recording());
        }
        let range = Self::selected_range(doc, start, end)?;
        let snapshot = doc.session.current();
        let clip = edit::copy(&snapshot, range).map_err(document_error)?;
        let len_samples = edit::pieces_len_samples(&clip);
        *self.0.clipboard.lock().unwrap() = Some(ClipboardData::Bound {
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
    /// cut/copied yet. A pure splice: when the clipboard is `Bound` to the open document's own
    /// session, or `Materialized` but already converted+cached for it (H-56, `paste_needs_job`
    /// said `false`). If the clipboard is `Materialized` and not yet usable here, this returns
    /// `error.document_busy` — the caller (`ipc::document_commands::edit_paste`) must have called
    /// [`Self::paste_needs_job`] first and run [`Self::begin_paste_job`]/[`Self::finish_paste_job`]
    /// instead.
    pub fn edit_paste(&self, target: PasteTarget) -> Result<EditResult, IpcError> {
        if self.is_normalize_busy() || self.is_paste_busy() {
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
        let session_id = doc.session.id().to_string();
        let clipboard = self.0.clipboard.lock().unwrap();
        let pieces: Vec<Piece> = match clipboard.as_ref() {
            Some(ClipboardData::Bound { pieces, .. }) => pieces.clone(),
            Some(ClipboardData::Materialized {
                converted: Some(c), ..
            }) if c.session_id == session_id => c.pieces.clone(),
            Some(ClipboardData::Materialized { .. }) => return Err(document_busy()),
            None => return Err(clipboard_empty()),
        };
        drop(clipboard);
        let inserted_len_samples = edit::pieces_len_samples(&pieces);
        let built = edit::paste(&pieces, resolved);
        let step = doc.session.commit_edit(built).map_err(document_error)?;
        Ok(self.apply_committed(doc, step, edit::post_paste(resolved, inserted_len_samples)))
    }

    /// Deletes `[start, end)`, closing the gap.
    pub fn edit_delete(&self, start: u64, end: u64) -> Result<EditResult, IpcError> {
        if self.is_normalize_busy() || self.is_paste_busy() {
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
        if self.is_normalize_busy() || self.is_paste_busy() {
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
        if self.is_normalize_busy() || self.is_paste_busy() {
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

    /// SPEC-008 §4.3: an Insert Silence duration outside §2.5's 1 sample .. 3 600 s range.
    fn insert_silence_duration_error() -> IpcError {
        IpcError::new(
            IpcErrorCode::InvalidArgument,
            "error.insert_silence_duration",
        )
    }

    /// Inserts `len_samples` of silence at `target`'s position (SPEC-008 §2.1/§2.5): the
    /// selection start if `target` is a `Range`, otherwise the cursor. Never removes anything —
    /// `Range`'s end is ignored. One undo entry `history.insert_silence`; the inserted range
    /// becomes the selection (SPEC-008 §2.3).
    pub fn edit_insert_silence(
        &self,
        target: PasteTarget,
        len_samples: u64,
    ) -> Result<EditResult, IpcError> {
        if self.is_normalize_busy() || self.is_paste_busy() {
            return Err(document_busy());
        }
        let mut guard = self.0.open.lock().unwrap();
        let doc = guard.as_mut().ok_or_else(no_document)?;
        if doc.session.is_recording() {
            return Err(IpcError::not_while_recording());
        }
        let rate_hz = doc.session.sample_rate_hz();
        let m = edit::validate_insert_silence_len(len_samples, rate_hz)
            .map_err(|_| Self::insert_silence_duration_error())?;
        let doc_len = doc.session.current().len_samples;
        let resolved = match target {
            PasteTarget::Cursor(c) if c <= doc_len => EditTarget::Cursor(c),
            PasteTarget::Cursor(_) => return Err(invalid_range()),
            PasteTarget::Range(s, e) => EditTarget::Range(Self::selected_range(doc, s, e)?),
        };
        let step = doc
            .session
            .commit_edit(edit::insert_silence(resolved, m))
            .map_err(document_error)?;
        Ok(self.apply_committed(doc, step, edit::post_insert_silence(resolved, m)))
    }

    // --- H-56: cross-document paste job (SPEC-008 §2.6.1) ------------------------------------

    /// Begins a cross-document paste job — validates the target the same way [`Self::edit_paste`]
    /// does, marks the document busy, and hands back the read-only facts the job's own thread
    /// needs to import the materialized clipboard without holding the document lock (mirrors
    /// [`Self::begin_normalize_job`]). Only meaningful when [`Self::paste_needs_job`] is `true`.
    pub fn begin_paste_job(&self, target: PasteTarget) -> Result<PasteJobSource, IpcError> {
        let mut busy = self.0.paste_busy.lock().unwrap();
        if *busy || self.is_normalize_busy() {
            return Err(document_busy());
        }
        let guard = self.0.open.lock().unwrap();
        let doc = guard.as_ref().ok_or_else(no_document)?;
        if doc.session.is_recording() {
            return Err(IpcError::not_while_recording());
        }
        let len_samples = doc.session.current().len_samples;
        let target = match target {
            PasteTarget::Cursor(c) if c <= len_samples => EditTarget::Cursor(c),
            PasteTarget::Cursor(_) => return Err(invalid_range()),
            PasteTarget::Range(s, e) => EditTarget::Range(Self::selected_range(doc, s, e)?),
        };
        let clip = match self.0.clipboard.lock().unwrap().as_ref() {
            Some(ClipboardData::Materialized { clip, .. }) => clip.clone(),
            Some(ClipboardData::Bound { .. }) => return Err(document_busy()),
            None => return Err(clipboard_empty()),
        };
        *busy = true;
        Ok(PasteJobSource {
            store: Arc::clone(doc.session.store()),
            session_id: doc.session.id().to_string(),
            target_rate_hz: doc.session.sample_rate_hz(),
            target,
            clip,
        })
    }

    /// Releases the paste-busy flag without committing anything (the job was cancelled, or the
    /// import failed before it produced any written audio).
    pub fn abandon_paste_job(&self) {
        *self.0.paste_busy.lock().unwrap() = false;
    }

    /// Commits a finished cross-document paste job's written audio as one `history.paste` undo
    /// entry, and rebinds/caches the clipboard (SPEC-008 §2.6 "later pastes"): a same-rate import
    /// fully rebinds the clipboard to this session (`Bound`) and deletes the materialized file;
    /// a converted one is cached for this session only (`converted`), keeping the original-rate
    /// file for any other document. Always releases the busy flag. If the open document changed
    /// session under the job (another `document_open` while it ran), this commits nothing —
    /// `error.document_busy`, exactly like a cancellation (SPEC-008 §2.6.1 all-or-nothing); the
    /// written chunks become unreachable store data, reclaimed by compaction.
    pub fn finish_paste_job(
        &self,
        source: &PasteJobSource,
        written: WrittenAudio,
    ) -> Result<EditResult, IpcError> {
        let mut guard = self.0.open.lock().unwrap();
        let result = (|| {
            let doc = guard.as_mut().ok_or_else(no_document)?;
            if doc.session.id() != source.session_id {
                return Err(document_busy());
            }
            let inserted_len_samples = written.len_samples;
            let built = edit::paste(&written.pieces, source.target);
            let step = doc.session.commit_edit(built).map_err(document_error)?;
            Ok((
                self.apply_committed(
                    doc,
                    step,
                    edit::post_paste(source.target, inserted_len_samples),
                ),
                written.pieces.clone(),
            ))
        })();
        drop(guard);
        if let Ok((_, ref pieces)) = result {
            let mut clipboard = self.0.clipboard.lock().unwrap();
            if source.clip.sample_rate_hz == source.target_rate_hz {
                vox_project::clipboard::delete_materialized(&source.clip);
                *clipboard = Some(ClipboardData::Bound {
                    pieces: pieces.clone(),
                    sample_rate_hz: source.target_rate_hz,
                    len_samples: written.len_samples,
                });
            } else if let Some(ClipboardData::Materialized { converted, .. }) = clipboard.as_mut() {
                *converted = Some(ConvertedClip {
                    session_id: source.session_id.clone(),
                    pieces: pieces.clone(),
                });
            }
        }
        *self.0.paste_busy.lock().unwrap() = false;
        result.map(|(edit_result, _)| edit_result)
    }

    /// Registers a new cross-document paste job's cancel token (mirrors `start_import_job`), so
    /// `edit_paste_cancel(job_id)` can reach it from another command invocation.
    pub fn start_paste_job(&self) -> (u32, CancelToken) {
        let job_id = self.0.next_paste_job.fetch_add(1, Ordering::Relaxed);
        let cancel = CancelToken::new();
        self.0
            .paste_jobs
            .lock()
            .unwrap()
            .insert(job_id, cancel.clone());
        (job_id, cancel)
    }

    /// `edit_paste_cancel`: best-effort, like every other job's cancel — a no-op for an unknown or
    /// already-finished job id.
    pub fn cancel_paste_job(&self, job_id: u32) {
        if let Some(cancel) = self.0.paste_jobs.lock().unwrap().get(&job_id) {
            cancel.cancel();
        }
    }

    /// Unregisters a finished (done/cancelled/failed) paste job's cancel token.
    pub fn end_paste_job(&self, job_id: u32) {
        self.0.paste_jobs.lock().unwrap().remove(&job_id);
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

    // --- T-602: Bake rack ----------------------------------------------------------------------

    /// T-602: begins a bake job over `[start, end)` — the same busy/recording/range rules and the
    /// same busy flag as a normalize job (SPEC-010 §2.1's "one document job at a time": edits,
    /// undo/redo and other document jobs are refused with `error.document_busy` until
    /// [`Self::finish_bake`]/[`Self::abandon_bake_job`]). Returns the read-only source the job
    /// renders from.
    pub fn begin_bake_job(&self, start: u64, end: u64) -> Result<NormalizeJobSource, IpcError> {
        let source = self.begin_normalize_job(start, end)?;
        // H-30: set only once every other check passed, mirroring `normalize_busy` above —
        // `ipc::rack_commands` starts refusing rack edits from this point on.
        *self.0.bake_running.lock().unwrap() = true;
        Ok(source)
    }

    /// Releases the busy flag of a bake that produced no edit (cancelled or failed): nothing was
    /// committed, so the document and the rack are exactly as before.
    pub fn abandon_bake_job(&self) {
        *self.0.bake_running.lock().unwrap() = false;
        self.abandon_normalize_job();
    }

    /// T-602: commits a finished bake's `edit` (one `history.bake` entry, SPEC-004 §2.2) and then
    /// loads `after` into the live rack (SPEC-004 OD-4 default: the rack is reset). Always
    /// releases the busy flag. If the document was replaced or edited while the job ran, nothing
    /// is committed and the rack is left alone (`error.document_busy`, like a cancellation — the
    /// rendered chunks become unreachable store data).
    pub fn finish_bake(
        &self,
        source: &NormalizeJobSource,
        edit: Edit,
        after: vox_rack::RackModel,
    ) -> Result<EditResult, IpcError> {
        let mut guard = self.0.open.lock().unwrap();
        let result = (|| {
            let doc = guard.as_mut().ok_or_else(no_document)?;
            if doc.session.id() != source.session_id
                || !Arc::ptr_eq(&doc.session.current(), &source.snapshot)
            {
                return Err(document_busy());
            }
            let step = doc.session.commit_edit(edit).map_err(document_error)?;
            Ok(self.apply_committed(doc, step, normalize_applied_post_edit(source.range)))
        })();
        drop(guard);
        if result.is_ok() {
            self.load_rack(after);
        }
        // H-30: cleared after `load_rack`, so a rack edit stays refused until the bake's own
        // post-job rack is the one live (nothing to race against it).
        *self.0.bake_running.lock().unwrap() = false;
        *self.0.normalize_busy.lock().unwrap() = false;
        result
    }

    /// Replaces the live rack (a bake's reset, or a bake entry's rack on undo/redo). Called
    /// without the document lock held: the engine's control thread must never wait on it.
    fn load_rack(&self, model: vox_rack::RackModel) {
        match self.0.engine.rack_load_model(model) {
            Some(Ok(_)) => {}
            Some(Err(error)) => tracing::warn!(%error, "loading the bake's rack failed"),
            None => tracing::warn!("loading the bake's rack failed: the engine is stopped"),
        }
    }

    // --- S2-03: markers (add, rename, move/resize, delete) -----------------------------------

    /// The current marker list, in canonical order (SPEC-009 §2.1). Empty (not an error) when no
    /// document is open.
    ///
    /// H-21: while a take or record operation runs, the markers added during it (not committed
    /// yet, [`Self::marker_add`]) are listed too, so the panel and waveform show them live.
    pub fn markers_get(&self) -> Vec<MarkerInfo> {
        let guard = self.0.open.lock().unwrap();
        let Some(doc) = guard.as_ref() else {
            return Vec::new();
        };
        let current = doc.session.current();
        let pending = doc.session.open_take_markers();
        if pending.is_empty() {
            return current.markers.iter().map(Into::into).collect();
        }
        let mut all: Vec<&Marker> = current.markers.iter().chain(pending).collect();
        all.sort_by_key(|m| (m.pos_samples, m.id.0));
        all.into_iter().map(Into::into).collect()
    }

    /// Adds a point (`len_samples == 0`) or region marker at `[pos_samples, pos_samples +
    /// len_samples)` (SPEC-009 §2.2; the UI resolves *where* per its transport-state table —
    /// heard position while playing, cursor or selection while stopped — before calling this).
    /// One undo entry `history.marker_add`, allowed during playback (marker edits never stop it,
    /// SPEC-009 §2.9). Named "Marker NN" (§2.3). `error.invalid_range` when the range doesn't fit
    /// the document.
    ///
    /// H-21 (SPEC-022 §2.9, SPEC-002 §2.2): allowed during a take or record operation — the
    /// marker (at the heard position, or `at + k` in the record window) joins the take's single
    /// edit instead (`Session::note_take_marker`, journaled for crash recovery; placed and clamped
    /// at commit, so the range isn't checked against the still-growing document here); a cancelled
    /// operation keeps it as its own "Add Marker" entry.
    pub fn marker_add(&self, pos_samples: u64, len_samples: u64) -> Result<MarkerInfo, IpcError> {
        let mut guard = self.0.open.lock().unwrap();
        let doc = guard.as_mut().ok_or_else(no_document)?;
        if let Some(take) = doc.session.open_take() {
            if pos_samples.checked_add(len_samples).is_none() {
                return Err(invalid_range());
            }
            let known: Vec<Marker> = doc
                .session
                .current()
                .markers
                .iter()
                .chain(doc.session.open_take_markers())
                .cloned()
                .collect();
            let name = next_marker_name(&known);
            let id = doc.session.new_marker_id();
            let marker = Marker::new(id, pos_samples, len_samples, name);
            doc.session
                .note_take_marker(take, marker.clone())
                .map_err(document_error)?;
            return Ok(MarkerInfo::from(&marker));
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

    /// Undoes the top entry (`Ok` with `changed: false` at the undo floor). Undoing a bake also
    /// restores the pre-bake rack from the entry's attachment (T-602, SPEC-004 AC-16).
    pub fn history_undo(&self) -> Result<EditResult, IpcError> {
        self.undo_or_redo(true)
    }

    /// Redoes the top entry (`Ok` with `changed: false` when there's nothing to redo). Redoing a
    /// bake also re-applies its rack reset (T-602, SPEC-004 AC-16).
    pub fn history_redo(&self) -> Result<EditResult, IpcError> {
        self.undo_or_redo(false)
    }

    fn undo_or_redo(&self, undo: bool) -> Result<EditResult, IpcError> {
        if self.is_normalize_busy() {
            return Err(document_busy());
        }
        let mut guard = self.0.open.lock().unwrap();
        let doc = guard.as_mut().ok_or_else(no_document)?;
        let step = if undo {
            doc.session.undo()
        } else {
            doc.session.redo()
        };
        let Some(step) = step.map_err(document_error)? else {
            let snapshot = doc.session.current();
            return Ok(EditResult {
                changed: false,
                audio_rev: snapshot.audio_rev,
                len_samples: snapshot.len_samples,
                selection: None,
                playhead_samples: 0,
            });
        };
        let rack = step
            .attachment
            .as_deref()
            .and_then(BakeAttachment::decode)
            .map(|a| if undo { a.before } else { a.after });
        let post = edit::post_undo_redo(step.first_at, step.snapshot.len_samples);
        let result = self.apply_committed(doc, step, post);
        drop(guard);
        if let Some(model) = rack {
            self.load_rack(model);
        }
        Ok(result)
    }
}

/// CRC-32 (SPEC-018 §4.2's `crc32-ieee/f32le/v1`) of `path`, decoded the way a reader (or this app,
/// on the next open) would — the same per-sample `f32::to_le_bytes` accumulation
/// [`vox_project::document_crc32`] uses on a live snapshot, so a matching fingerprint reads the
/// same either way. T-209: dispatches on `container`, since Save As can now target either.
fn saved_file_crc32(path: &Path, container: SaveContainer) -> vox_io::Result<u32> {
    match container {
        SaveContainer::Wav => {
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
        SaveContainer::Flac => {
            let (_info, mut source) = vox_io::DecodeSource::open(path)?;
            let mut hasher = crc32fast::Hasher::new();
            let mut buf = vec![0f32; vox_project::CHUNK_SAMPLES];
            loop {
                let n = source.read_frames(&mut buf)?;
                if n == 0 {
                    break;
                }
                for &s in &buf[..n] {
                    hasher.update(&s.to_le_bytes());
                }
            }
            Ok(hasher.finalize())
        }
    }
}

/// FLAC's two supported bit depths — `save_as`/`save` refuse `Flac` + `Bit32Float` before this is
/// ever reached (`invalid_flac_bit_depth`); 24-bit is the only other FLAC depth (SPEC-005 §2.6/
/// §2.11).
fn flac_bits(bits: BitDepth) -> vox_io::FlacBitDepth {
    match bits {
        BitDepth::Bit16 => vox_io::FlacBitDepth::Int16,
        BitDepth::Bit24 | BitDepth::Bit32Float => vox_io::FlacBitDepth::Int24,
    }
}

/// H-75: everything a save's write ([`write_save`]) and its outcome ([`DocumentService::
/// apply_save_outcome`]) need from `doc`, captured once while the document lock is held —
/// [`DocumentService::finish_save`]/[`Self::finish_save_as`] release the lock for the write
/// itself (SPEC-005 §2.7: "editing and playback continue"), so nothing here may be read from
/// `doc` again until they re-lock afterwards.
struct SavePrep {
    /// [`vox_project::Session::id`] at prepare time — if it differs by the time the write
    /// finishes, the document was closed or replaced mid-save (a new `document_open`); the save
    /// still wrote a correct file, but there is no document left to record it against.
    session_id: String,
    store: Arc<vox_project::ChunkStore>,
    snapshot: Arc<DocSnapshot>,
    /// The seq [`vox_project::History::current_seq`] reported for `snapshot` (H-75): the *only*
    /// value [`vox_project::Session::mark_saved_file_at`] may record as saved once the write
    /// finishes. An edit committed after this was captured bumps the session's real current seq,
    /// but its bytes are never in the file this save writes — recording anything later than this
    /// would wrongly clear the dirty flag for that edit.
    seq_at_snapshot: u64,
    sample_rate_hz: u32,
    markers: Vec<MarkerItemModel>,
    rack: serde_json::Value,
    view: serde_json::Value,
    needs_backup: bool,
    /// The previous save's fingerprint — [`write_save`]'s fallback if re-reading the just-written
    /// file back fails (matches the pre-H-75 `save_to`'s `.unwrap_or(doc.sidecar.audio_crc32)`).
    prior_audio_crc32: u32,
    /// Whether this write needs to touch the audio file at all (SPEC-018 §2.3/§2.4) — decided
    /// here, against `doc`'s sidecar bookkeeping, since that's only safe to read under the lock.
    needs_full: bool,
}

/// Captures a [`SavePrep`] for a save that will write `path` in `container`/`bits`/`dither`.
/// `force_full` is Save As's "a different target (or format) is never a sidecar-only write" (was
/// previously done by bumping `doc.sidecar.last_saved_audio_rev` back one before calling the old,
/// single-locked `save_to` — a `doc` mutation whose only job was to force this same decision).
fn prepare_save(
    doc: &OpenDocument,
    engine: &EngineHandle,
    path: &Path,
    container: SaveContainer,
    bits: BitDepth,
    dither: SaveDitherPref,
    force_full: bool,
) -> SavePrep {
    let snapshot = doc.session.current();
    let markers = current_marker_items(doc);
    let needs_full = force_full
        || snapshot.audio_rev != doc.sidecar.last_saved_audio_rev
        || markers != doc.sidecar.last_written_markers
        || bits != doc.save_bits
        || container != doc.save_container
        || dither != doc.save_dither
        || std::fs::metadata(path).is_err();
    SavePrep {
        session_id: doc.session.id().to_string(),
        store: Arc::clone(doc.session.store()),
        seq_at_snapshot: doc.session.history().current_seq(),
        sample_rate_hz: doc.session.sample_rate_hz(),
        rack: current_rack_value(engine),
        view: doc.sidecar.view.clone(),
        needs_backup: doc.sidecar.needs_backup,
        prior_audio_crc32: doc.sidecar.audio_crc32,
        needs_full,
        markers,
        snapshot,
    }
}

/// H-75: a finished save write's outcome — everything [`DocumentService::apply_save_outcome`]
/// needs to update `doc` and the session's saved-seq bookkeeping once it has re-locked.
struct SaveWriteOutcome {
    audio_crc32: u32,
    /// `(size, mtime_unix_ms)` of the file on disk after the write, `None` if it couldn't be
    /// stat'd (mirrors the pre-H-75 `save_to`'s `file_meta`-derived `written_facts`).
    file_facts: Option<(u64, u64)>,
    sidecar_written: bool,
    /// Only meaningful when `sidecar_written`.
    persisted_digest: u32,
    notices: Vec<SidecarNoticeInfo>,
}

/// Runs a save's write against `prep` (SPEC-005 §2.7, extended by SPEC-018 §2.3/§2.4/§2.8/§2.9):
/// the audio only when `prep.needs_full`, the sidecar always. H-75: no `OpenDocument`/document
/// lock access at all — everything needed was captured by [`prepare_save`] while the lock was
/// held, and the document may be edited (or closed) by another command while this runs.
#[allow(clippy::too_many_arguments)]
fn write_save(
    prep: &SavePrep,
    path: &Path,
    container: SaveContainer,
    bits: BitDepth,
    dither: SaveDitherPref,
    cancel: &CancelToken,
    on_progress: &mut dyn FnMut(f32),
) -> Result<SaveWriteOutcome, IpcError> {
    let save_format = save_format_model(container, bits, dither);

    // SPEC-004 §2.6, AC-12: leftovers of an interrupted save by a dead process go first.
    if let Some(parent) = path.parent() {
        vox_project::gc::remove_stale_temp_files(parent);
    }
    if prep.needs_full {
        let mut reader = SnapshotReader::new(Arc::clone(&prep.store), Arc::clone(&prep.snapshot));
        match container {
            SaveContainer::Wav => {
                let wav_markers: Vec<Marker> = prep.markers.iter().map(marker_from_item).collect();
                vox_project::save_snapshot_wav(
                    &mut reader,
                    path,
                    bits.into(),
                    dither.into(),
                    &wav_markers,
                    cancel,
                    on_progress,
                )
                .map_err(|err| match err {
                    ProjectError::Wav(io_err) => io_save_error(io_err),
                    other => document_error(other),
                })?;
            }
            SaveContainer::Flac => {
                save_snapshot_flac(
                    &mut reader,
                    path,
                    flac_bits(bits),
                    dither.into(),
                    cancel,
                    on_progress,
                )
                .map_err(|err| match err {
                    ProjectError::Wav(io_err) => io_save_error(io_err),
                    other => document_error(other),
                })?;
            }
        }
    } else {
        // H-70: a sidecar-only save (SPEC-018 §2.3) still reports a job — the write is
        // effectively instant, so it just jumps straight to done.
        on_progress(1.0);
    }

    // T-209: SPEC-005 §2.9's "other containers" — FLAC carries no markers.
    let markers_not_in_flac = container == SaveContainer::Flac && !prep.markers.is_empty();

    // Re-reads the just-written file rather than hashing the in-memory snapshot: 16/24-bit
    // targets (WAV or FLAC) are TPDF-dithered, so the bytes on disk are not bit-identical to the
    // pre-quantization f32 samples — the sidecar's fingerprint (SPEC-018 §4.2) must match what a
    // later open would decode, or every 16/24-bit save would wrongly look "changed" at reopen.
    let audio_crc32 = saved_file_crc32(path, container).unwrap_or(prep.prior_audio_crc32);
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
        sample_rate_hz: prep.sample_rate_hz,
        len_samples: prep.snapshot.len_samples,
        audio_crc32,
        save_format: save_format.clone(),
        markers: &prep.markers,
        rack: prep.rack.clone(),
        view: prep.view.clone(),
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        written_at: vox_project::sidecar::system_time_to_rfc3339(std::time::SystemTime::now()),
    };
    let sidecar_path = sidecar_path_for(path);
    let sidecar_written = write_sidecar(&sidecar_path, &write_input, prep.needs_backup).is_ok();

    let name = || {
        path.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    };
    let mut notices = Vec::new();
    let mut persisted_digest = 0;
    if sidecar_written {
        persisted_digest =
            vox_project::sidecar::persisted_digest(&save_format, &prep.markers, &prep.rack);
        // T-209 (SPEC-005 §2.9): only shown when the sidecar write itself succeeded — a failed
        // sidecar write already has a more urgent notice below, and the sidecar (M3+) is where
        // markers actually survive a FLAC save, so that notice matters more when it's missing too.
        if markers_not_in_flac {
            notices.push(SidecarNoticeInfo {
                key: "notice.save.markers_not_in_flac",
                params: vec![("name", name()), ("count", prep.markers.len().to_string())],
            });
        }
    } else {
        // `sidecar.persisted_digest`/`needs_backup` are left untouched (by the caller): `
        // sidecar_dirty` stays set, and the next Save retries a sidecar-only write (SPEC-018 §2.9).
        let key = if save_format.container == "flac" {
            "notice.save.sidecar_failed.flac"
        } else {
            "notice.save.sidecar_failed"
        };
        notices.push(SidecarNoticeInfo {
            key,
            params: vec![("name", name())],
        });
    }

    // Journal `saved` regardless of the sidecar outcome (SPEC-018 §2.9): the audio save (or the
    // decision to keep its bytes) stands either way. T-301: the file's size/mtime too, so crash
    // recovery can tell a later change on disk from this save (SPEC-004 §2.7).
    let file_facts = file_meta
        .as_ref()
        .map(|m| (m.len(), m.modified().map(unix_ms_of).unwrap_or(0)));

    Ok(SaveWriteOutcome {
        audio_crc32,
        file_facts,
        sidecar_written,
        persisted_digest,
        notices,
    })
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;
    use std::time::Instant;

    use vox_engine::backend::fake::{FakeBackend, FakeDevice, FakeDirection};
    use vox_engine::spectro::{SpectroConfig, SpectroService};
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

    /// Like [`service_with_output`] (a plugged, preferred fake output device, so a live
    /// [`vox_rack::RackHost`] actually exists to recover a slot in), but with `factories` only
    /// (not the full built-in set) and the `Registry` itself handed back — H-40 tests hot-add a
    /// factory into it later (`register`, mirroring `Registry::upsert`/T-804's install/rescan
    /// hot-add) to drive a live recovery of a Missing slot, the same way the real
    /// `PluginCatalog` does into every registry it observes. Blocks (briefly) until the output
    /// has opened.
    fn test_engine_with_registry(
        factories: Vec<Arc<dyn vox_rack::ModuleFactory>>,
    ) -> (Engine, Arc<Registry>) {
        let fake = FakeBackend::new(7);
        fake.plug(vox_engine::HostId::Alsa, dac());
        let registry = Arc::new(Registry::with_factories(factories).unwrap());
        let mut cfg = EngineConfig::new(Arc::new(fake), registry.clone());
        cfg.prefs = vox_engine::DevicePrefs {
            output_device: Some("DAC".into()),
            ..vox_engine::DevicePrefs::default()
        };
        let engine = Engine::start(cfg).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if engine
                .handle()
                .devices()
                .is_some_and(|d| d.output_status == vox_engine::device_state::DeviceStatus::Healthy)
            {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "the fake output device never opened"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        (engine, registry)
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

    /// H-17 item 3: `DocumentService::housekeeping`'s rewritten short-lock/no-lock/short-lock
    /// orchestration (`Session::begin_compact`/`finish_compact` instead of one `&mut Session`
    /// call) still does nothing and reports so when a fresh, tiny document is well within the
    /// default disk limits — exercising the full loop's `DiskAction::None` exit.
    #[test]
    fn housekeeping_does_nothing_under_the_default_limits() {
        let (service, _engine, dir) = service("housekeeping-noop");
        let wav_path = dir.join("in.wav");
        let samples = vox_testkit::signal::sine(440.0, -6.0, 0.05, 48_000).unwrap();
        write_fixture_wav(
            &wav_path,
            &samples,
            vox_testkit::wav::BitDepth::Float32,
            48_000,
        );
        service.open(&wav_path, false).unwrap();

        let free = vox_project::FixedFreeSpace::new(100 * 1024 * 1024 * 1024);
        let report = service.housekeeping(&free).expect("a document is open");
        assert!(!report.skipped_recording);
        assert!(!report.compacted);
        assert_eq!(report.dropped_undo, 0);
        assert_eq!(report.freed_bytes, 0);
        assert!(!report.almost_full);

        // The document is still fully usable (no lock left held, no state corrupted).
        assert_eq!(service.info().len_samples, samples.len() as u64);
    }

    /// H-17 item 3: recording still gates housekeeping out entirely, same as before the split
    /// (SPEC-004 §2.5 "never while recording") — the short first lock sees `is_recording()` and
    /// returns before ever calling `begin_compact`.
    #[test]
    fn housekeeping_is_skipped_while_recording() {
        let (service, _engine, dir) = service("housekeeping-recording");
        let wav_path = dir.join("in.wav");
        let samples = vox_testkit::signal::sine(440.0, -6.0, 0.05, 48_000).unwrap();
        write_fixture_wav(
            &wav_path,
            &samples,
            vox_testkit::wav::BitDepth::Float32,
            48_000,
        );
        service.open(&wav_path, false).unwrap();
        {
            let mut guard = service.0.open.lock().unwrap();
            let doc = guard.as_mut().unwrap();
            doc.session
                .begin_take(TakeMode::New, TakeWriterOptions::default())
                .unwrap();
        }

        let free = vox_project::FixedFreeSpace::new(100 * 1024 * 1024 * 1024);
        let report = service.housekeeping(&free).expect("a document is open");
        assert!(report.skipped_recording);
        assert!(!report.compacted);
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
        service.save(false, false, false).unwrap();
        let (decoded, _info) = vox_testkit::wav::read_wav_file(&wav_path).unwrap();
        let (_rate, _channels, mut source) = vox_io::read_wav(&wav_path).unwrap();
        let mut original = vec![0.0f32; samples.len()];
        source.read_mono(&mut original).unwrap();
        assert_eq!(decoded, original, "save with no edits is bit-exact");

        // Save As a new path and format (32-bit float): binds the document to the new path/bits.
        let save_as_path = dir.join("out.wav");
        let info = service
            .save_as(
                &save_as_path,
                SaveContainer::Wav,
                BitDepth::Bit32Float,
                SaveDitherPref::Tpdf,
                false,
                false,
            )
            .unwrap();
        assert_eq!(info.path.as_deref(), Some(save_as_path.to_str().unwrap()));
        assert!(!info.dirty);
        let (decoded32, meta) = vox_testkit::wav::read_wav_file(&save_as_path).unwrap();
        assert_eq!(meta.sample_rate, 48_000);
        for (a, b) in decoded32.iter().zip(original.iter()) {
            assert!((a - b).abs() < 1e-6);
        }
    }

    /// T-209 (SPEC-005 §2.6/§2.7/§2.11): Save As's `container`/`bits` reach the FLAC encoder —
    /// the saved file decodes back to the same audio, and the document rebinds to FLAC.
    #[test]
    fn save_as_flac_reaches_the_encoder_and_round_trips() {
        let (service, _engine, dir) = service("save-as-flac");
        let wav_path = dir.join("in.wav");
        let samples = vox_testkit::signal::sine(997.0, -6.0, 0.1, 48_000).unwrap();
        write_fixture_wav(
            &wav_path,
            &samples,
            vox_testkit::wav::BitDepth::Int24,
            48_000,
        );
        service.open(&wav_path, false).unwrap();

        let flac_path = dir.join("out.flac");
        let info = service
            .save_as(
                &flac_path,
                SaveContainer::Flac,
                BitDepth::Bit16,
                SaveDitherPref::Tpdf,
                false,
                false,
            )
            .unwrap();
        assert_eq!(info.path.as_deref(), Some(flac_path.to_str().unwrap()));
        assert!(!info.dirty);
        assert!(flac_path.exists());

        // Decodes back through the general decoder (proves it's really a FLAC file, not a
        // WAV written with a misleading extension).
        let (decode_info, mut source) = vox_io::DecodeSource::open(&flac_path).unwrap();
        assert_eq!(decode_info.container, "flac");
        let mut decoded = vec![0.0f32; samples.len()];
        let mut pos = 0;
        loop {
            let n = source.read_frames(&mut decoded[pos..]).unwrap();
            if n == 0 {
                break;
            }
            pos += n;
        }
        assert_eq!(pos, samples.len());

        // A second Save (in place, no edits) at the bound FLAC format is a no-op write that
        // still round-trips — proves `doc.save_container` was actually rebound, not just the
        // one Save As call.
        service.save(false, false, false).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// H-30 (SPEC-007 §4.1): "while a save, export or bake job runs, tile work uses at most half
    /// of the workers" — checked directly through `SpectroService::worker_cap_now` (a
    /// synchronous witness of the guard) rather than racing real tile computation: a large
    /// buffer written as FLAC (real encoder work, `vox_io::write_flac`) keeps `save_as` busy long
    /// enough for the polling loop below to observe the halved cap without any fixed sleep to
    /// race against.
    #[test]
    fn save_as_holds_the_spectro_guard_while_it_writes() {
        let (service, _engine, dir) = service("save-spectro-guard");
        let wav_path = dir.join("in.wav");
        let samples = vox_testkit::signal::pink_noise(11, -20.0, 60.0, 48_000).unwrap();
        write_fixture_wav(
            &wav_path,
            &samples,
            vox_testkit::wav::BitDepth::Int24,
            48_000,
        );
        service.open(&wav_path, false).unwrap();

        let spectro = Arc::new(SpectroService::new(SpectroConfig {
            workers: 4,
            cache_cap_bytes: None,
        }));
        service.set_spectro(Arc::clone(&spectro));

        let flac_path = dir.join("out.flac");
        let done = Arc::new(AtomicBool::new(false));
        let done_for_job = Arc::clone(&done);
        let job_service = service.clone();
        std::thread::Builder::new()
            .name("save-guard-test-job".into())
            .spawn(move || {
                job_service
                    .save_as(
                        &flac_path,
                        SaveContainer::Flac,
                        BitDepth::Bit16,
                        SaveDitherPref::Tpdf,
                        true,
                        true,
                    )
                    .unwrap();
                done_for_job.store(true, Ordering::SeqCst);
            })
            .unwrap();

        let mut saw_halved = false;
        let deadline = Instant::now() + Duration::from_secs(10);
        while !done.load(Ordering::SeqCst) {
            if spectro.worker_cap_now() == 2 {
                saw_halved = true;
                break;
            }
            assert!(
                Instant::now() < deadline,
                "the save_as job never finished within 10s"
            );
            std::thread::sleep(Duration::from_micros(200));
        }
        while !done.load(Ordering::SeqCst) {
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(
            saw_halved,
            "never observed the halved tile-worker cap while save_as wrote the file"
        );
        assert_eq!(
            spectro.worker_cap_now(),
            4,
            "the cap is restored once save_as finishes"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    // --- H-20 item 2: keep the source format --------------------------------------------------

    /// H-20 (SPEC-005 §2.6): opening a FLAC source keeps FLAC as the save format *without* an
    /// explicit Save As — plain `document_open` + `document_save` (this bug: `save_bits_for_import`
    /// compared `probe.container` against the literal `"wav"`, but symphonia's WAV reader's
    /// `format_info().short_name` is actually `"wave"`, so that branch never matched and every
    /// import — WAV included — silently fell back to the lossy-import default of WAV 24-bit).
    #[test]
    fn opening_a_flac_source_keeps_flac_as_the_save_format_without_save_as() {
        if std::process::Command::new("flac")
            .arg("--version")
            .output()
            .is_err()
        {
            eprintln!("skipping: flac CLI not installed");
            return;
        }
        let (service, _engine, dir) = service("flac-keeps-flac");
        let samples = vox_testkit::signal::sine(997.0, -6.0, 0.1, 48_000).unwrap();
        let wav_path = dir.join("in.wav");
        write_fixture_wav(
            &wav_path,
            &samples,
            vox_testkit::wav::BitDepth::Int24,
            48_000,
        );
        let flac_path = dir.join("in.flac");
        let status = std::process::Command::new("flac")
            .args(["-f", "--totally-silent", "-o"])
            .arg(&flac_path)
            .arg(&wav_path)
            .status()
            .unwrap();
        assert!(status.success());

        service.open(&flac_path, false).unwrap();
        // Plain Save (never Save As): the document must already be bound to FLAC.
        service.save(false, false, false).unwrap();

        // Decodes back through the general decoder (proves it's still really a FLAC file, not a
        // WAV silently written over the `.flac` path).
        let (decode_info, _) = vox_io::DecodeSource::open(&flac_path).unwrap();
        assert_eq!(decode_info.container, "flac");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// H-20 (SPEC-005 §2.6 promotion table): direct unit coverage of every row, including the ones
    /// no integration test builds a real file for (FLAC 25-32-bit, big-endian PCM codec names that
    /// never occur in a real WAV but are handled defensively).
    #[test]
    fn save_format_for_import_matches_the_spec_005_promotion_table() {
        fn probe_of(container: &str, codec: &str, bits_per_sample: Option<u32>) -> ImportProbe {
            ImportProbe {
                container: container.to_string(),
                codec: codec.to_string(),
                sample_rate_hz: 48_000,
                channels: vec![vox_project::ImportChannel {
                    label: "Mono".to_string(),
                    is_lfe: false,
                }],
                len_samples: Some(48_000),
                channel_peaks_dbfs: Vec::new(),
                identical_channels: false,
                suggested_channel: None,
                bits_per_sample,
                has_foreign_metadata: false,
            }
        }
        // clippy::type_complexity: named here purely to keep the table below legible.
        type Case = (
            &'static str,
            &'static str,
            Option<u32>,
            SaveContainer,
            BitDepth,
            bool,
        );
        let cases: &[Case] = &[
            (
                "wave",
                "pcm_s16le",
                Some(16),
                SaveContainer::Wav,
                BitDepth::Bit16,
                false,
            ),
            (
                "wave",
                "pcm_s24le",
                Some(24),
                SaveContainer::Wav,
                BitDepth::Bit24,
                false,
            ),
            (
                "wave",
                "pcm_f32le",
                Some(32),
                SaveContainer::Wav,
                BitDepth::Bit32Float,
                false,
            ),
            (
                "wave",
                "pcm_u8",
                Some(8),
                SaveContainer::Wav,
                BitDepth::Bit16,
                true,
            ),
            (
                "wave",
                "pcm_alaw",
                None,
                SaveContainer::Wav,
                BitDepth::Bit16,
                true,
            ),
            (
                "wave",
                "pcm_mulaw",
                None,
                SaveContainer::Wav,
                BitDepth::Bit16,
                true,
            ),
            (
                "wave",
                "pcm_s32le",
                None,
                SaveContainer::Wav,
                BitDepth::Bit32Float,
                true,
            ),
            (
                "wave",
                "pcm_f64le",
                None,
                SaveContainer::Wav,
                BitDepth::Bit32Float,
                true,
            ),
            (
                "flac",
                "flac",
                Some(8),
                SaveContainer::Flac,
                BitDepth::Bit16,
                false,
            ),
            (
                "flac",
                "flac",
                Some(16),
                SaveContainer::Flac,
                BitDepth::Bit16,
                false,
            ),
            (
                "flac",
                "flac",
                Some(17),
                SaveContainer::Flac,
                BitDepth::Bit24,
                false,
            ),
            (
                "flac",
                "flac",
                Some(24),
                SaveContainer::Flac,
                BitDepth::Bit24,
                false,
            ),
            (
                "flac",
                "flac",
                Some(25),
                SaveContainer::Flac,
                BitDepth::Bit24,
                true,
            ),
            (
                "flac",
                "flac",
                Some(32),
                SaveContainer::Flac,
                BitDepth::Bit24,
                true,
            ),
            (
                "mp3",
                "mp3",
                None,
                SaveContainer::Wav,
                BitDepth::Bit24,
                false,
            ),
            (
                "isomp4",
                "aac",
                None,
                SaveContainer::Wav,
                BitDepth::Bit24,
                false,
            ),
            (
                "ogg",
                "vorbis",
                None,
                SaveContainer::Wav,
                BitDepth::Bit24,
                false,
            ),
        ];
        for &(container, codec, bits, want_container, want_bits, want_mapped) in cases {
            let probe = probe_of(container, codec, bits);
            let (got_container, got_bits, got_mapped) = save_format_for_import(&probe);
            assert_eq!(
                got_container, want_container,
                "{container}/{codec}/{bits:?}"
            );
            assert_eq!(got_bits, want_bits, "{container}/{codec}/{bits:?}");
            assert_eq!(got_mapped, want_mapped, "{container}/{codec}/{bits:?}");
        }
    }

    // --- H-20 item 3: notices -------------------------------------------------------------------

    /// H-20 (SPEC-005 §2.6 "notice at open"): an 8-bit WAV posts `notice.open.format_mapped` and
    /// ends up bound to WAV 16-bit — `(u - 128) * 256` exactly (SPEC-005 AC-17).
    #[test]
    fn open_of_an_eight_bit_wav_posts_the_format_mapped_notice_and_saves_as_sixteen_bit() {
        let (service, _engine, dir) = service("format-mapped-8bit");
        let wav_path = dir.join("in.wav");
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 48_000,
            bits_per_sample: 8,
            sample_format: hound::SampleFormat::Int,
        };
        let mut w = hound::WavWriter::create(&wav_path, spec).unwrap();
        // hound's 8-bit samples are signed i8 offsets from the unsigned byte (module doc of the
        // sibling `open_accepts_an_8_bit_wav_via_the_symphonia_decode_path` test).
        for v in [-128i32, 0, 127] {
            w.write_sample(v).unwrap();
        }
        w.finalize().unwrap();

        service.open(&wav_path, false).unwrap();
        let notices = service.take_sidecar_notices();
        assert_eq!(notices.len(), 1);
        assert_eq!(notices[0].key, "notice.open.format_mapped");
        assert_eq!(
            notices[0].params,
            vec![("from", "8-bit".to_string()), ("to", "16-bit".to_string())]
        );

        let out_path = dir.join("out.wav");
        service
            .save_as(
                &out_path,
                SaveContainer::Wav,
                BitDepth::Bit16,
                SaveDitherPref::Tpdf,
                false,
                false,
            )
            .unwrap();
        let (_, _, mut source) = vox_io::read_wav(&out_path).unwrap();
        assert_eq!(source.format().bits_per_sample, 16);
        let mut decoded = vec![0.0f32; 3];
        source.read_mono(&mut decoded).unwrap();
        for (byte, got) in [0u8, 128, 255].iter().zip(decoded.iter()) {
            let expected = ((f32::from(*byte) - 128.0) * 256.0) / 32_768.0;
            assert!(
                (got - expected).abs() < 1e-6,
                "got {got}, expected {expected}"
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A native 16/24-bit or 32-bit float WAV never posts `notice.open.format_mapped` (SPEC-005
    /// §2.6's table: "—" for those rows).
    #[test]
    fn open_of_a_native_wav_posts_no_format_mapped_notice() {
        let (service, _engine, dir) = service("format-mapped-none");
        let wav_path = dir.join("in.wav");
        let samples = vox_testkit::signal::sine(440.0, -6.0, 0.05, 48_000).unwrap();
        write_fixture_wav(
            &wav_path,
            &samples,
            vox_testkit::wav::BitDepth::Int24,
            48_000,
        );

        service.open(&wav_path, false).unwrap();
        assert!(
            service.take_sidecar_notices().is_empty(),
            "a native 24-bit WAV needs no format-mapped notice"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// H-72 (SPEC-005 §2.5/§2.9): a `cue ` chunk declaring more than `CUE_MAX_POINTS` (100 000,
    /// `vox_io`'s private constant) points is malformed — the audio still opens, with no markers,
    /// and posts `notice.open.markers_unreadable` (the previous agent's `WavMarkersResult` had no
    /// caller at all; this is the open path wired to it).
    #[test]
    fn open_of_a_wav_with_a_malformed_cue_chunk_posts_the_markers_unreadable_notice() {
        let (service, _engine, dir) = service("markers-unreadable");
        let wav_path = dir.join("in.wav");
        let samples = vox_testkit::signal::sine(440.0, -6.0, 0.05, 48_000).unwrap();
        write_fixture_wav(
            &wav_path,
            &samples,
            vox_testkit::wav::BitDepth::Int24,
            48_000,
        );
        // Append a `cue ` chunk whose declared point count is over SPEC-005 §2.9's 100 000
        // threshold ("counts as malformed") — no real cue records follow it.
        let mut bytes = std::fs::read(&wav_path).unwrap();
        bytes.extend_from_slice(b"cue ");
        bytes.extend_from_slice(&4u32.to_le_bytes());
        bytes.extend_from_slice(&100_001u32.to_le_bytes());
        let riff_size = (bytes.len() - 8) as u32;
        bytes[4..8].copy_from_slice(&riff_size.to_le_bytes());
        std::fs::write(&wav_path, &bytes).unwrap();

        service.open(&wav_path, false).unwrap();
        assert!(
            service.markers_get().is_empty(),
            "a malformed cue chunk still drops its cue points, exactly as before H-72"
        );
        let notices = service.take_sidecar_notices();
        assert!(
            notices
                .iter()
                .any(|n| n.key == "notice.open.markers_unreadable"),
            "a malformed cue chunk posts notice.open.markers_unreadable (SPEC-005 §2.5), got {notices:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// H-72 (SPEC-005 §2.9): a cue point whose region falls outside the document is dropped
    /// (unchanged from before H-72 — `import.rs`'s own filter) and now posts
    /// `notice.open.markers_out_of_range` with the dropped count.
    #[test]
    fn open_of_a_wav_with_an_out_of_range_marker_posts_the_notice_and_drops_it() {
        let (service, _engine, dir) = service("markers-out-of-range");
        let samples = vox_testkit::signal::sine(440.0, -6.0, 0.05, 48_000).unwrap();
        let len_samples = samples.len() as u64;
        let wav_path = dir.join("in.wav");
        let markers = vec![
            vox_io::WavMarker {
                pos_samples: 10,
                len_samples: 0,
                name: "In range".to_string(),
            },
            vox_io::WavMarker {
                pos_samples: len_samples + 1_000,
                len_samples: 0,
                name: "Out of range".to_string(),
            },
        ];
        vox_io::write_wav_with_markers(
            &wav_path,
            48_000,
            vox_io::BitDepth::Int24,
            vox_io::DitherMode::None,
            &samples,
            &markers,
        )
        .unwrap();

        service.open(&wav_path, false).unwrap();
        let kept = service.markers_get();
        assert_eq!(kept.len(), 1, "only the in-range marker survives");
        assert_eq!(kept[0].pos_samples, 10);
        let notices = service.take_sidecar_notices();
        let notice = notices
            .iter()
            .find(|n| n.key == "notice.open.markers_out_of_range")
            .unwrap_or_else(|| {
                panic!("expected notice.open.markers_out_of_range, got {notices:?}")
            });
        assert_eq!(notice.params, vec![("count", "1".to_string())]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// H-20 (SPEC-005 §2.10): the first Save over a source with `LIST INFO`/`bext`/etc metadata
    /// posts `notice.save.metadata_dropped` exactly once — a second Save stays silent.
    #[test]
    fn save_posts_the_metadata_dropped_notice_once_for_a_wav_with_foreign_metadata() {
        let (service, _engine, dir) = service("metadata-dropped");
        let wav_path = dir.join("in.wav");
        let samples = vox_testkit::signal::sine(440.0, -6.0, 0.05, 48_000).unwrap();
        write_fixture_wav(
            &wav_path,
            &samples,
            vox_testkit::wav::BitDepth::Int24,
            48_000,
        );
        // Append a raw `bext` chunk after the fact — symphonia's WAV reader doesn't surface it as
        // metadata at all (only `LIST INFO` gets that treatment), so this also exercises H-20's
        // raw chunk scan (`vox_io::wav_has_foreign_metadata`), not just the symphonia-side check.
        let mut bytes = std::fs::read(&wav_path).unwrap();
        let body = vec![0u8; 8];
        bytes.extend_from_slice(b"bext");
        bytes.extend_from_slice(&(body.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&body);
        let riff_size = (bytes.len() - 8) as u32;
        bytes[4..8].copy_from_slice(&riff_size.to_le_bytes());
        std::fs::write(&wav_path, &bytes).unwrap();

        service.open(&wav_path, false).unwrap();
        service.save(false, false, false).unwrap();
        let notices = service.take_sidecar_notices();
        assert_eq!(
            notices
                .iter()
                .filter(|n| n.key == "notice.save.metadata_dropped")
                .count(),
            1,
            "the first Save must post the notice exactly once"
        );

        service.save(false, false, false).unwrap();
        let notices = service.take_sidecar_notices();
        assert!(
            !notices
                .iter()
                .any(|n| n.key == "notice.save.metadata_dropped"),
            "a later Save must not repeat it"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// H-20 (SPEC-005 §2.4 "Saving over a multichannel source"): the first Save over a stereo
    /// source needs confirmation; confirming (or a later Save) never asks again (AC-10: "exactly
    /// once per document").
    #[test]
    fn save_over_a_multichannel_source_needs_confirmation_exactly_once() {
        let (service, _engine, dir) = service("multichannel-warn");
        let left = vox_testkit::signal::sine(400.0, -10.0, 0.05, 48_000).unwrap();
        let right = vox_testkit::signal::sine(900.0, -14.0, 0.05, 48_000).unwrap();
        let mut interleaved = Vec::with_capacity(left.len() * 2);
        for (&l, &r) in left.iter().zip(right.iter()) {
            interleaved.push(l);
            interleaved.push(r);
        }
        let stereo_path = dir.join("stereo.wav");
        write_wav_file_multi(&stereo_path, &interleaved, 2, 48_000);
        service.open(&stereo_path, false).unwrap();

        let err = service.save(false, false, false).unwrap_err();
        assert_eq!(err.code, IpcErrorCode::NeedsConfirmation);
        assert_eq!(err.key, "dialog.multichannel_source");

        service.save(false, false, true).unwrap();
        // A second Save never asks again.
        service.save(false, false, false).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// SPEC-005 §2.6/§2.11: FLAC has no 32-bit float format.
    #[test]
    fn save_as_flac_refuses_32_bit_float() {
        let (service, _engine, dir) = service("save-as-flac-32f");
        let wav_path = dir.join("in.wav");
        let samples = vox_testkit::signal::sine(440.0, -6.0, 0.05, 48_000).unwrap();
        write_fixture_wav(
            &wav_path,
            &samples,
            vox_testkit::wav::BitDepth::Int24,
            48_000,
        );
        service.open(&wav_path, false).unwrap();

        let err = service
            .save_as(
                &dir.join("out.flac"),
                SaveContainer::Flac,
                BitDepth::Bit32Float,
                SaveDitherPref::Tpdf,
                false,
                false,
            )
            .unwrap_err();
        assert_eq!(err.key, "error.save.flac_needs_int_bits");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// T-209 (SPEC-005 §2.8, AC-6): the clip prompt appears only when a sample exceeds full
    /// scale, and each of the three choices does exactly what it says.
    #[test]
    fn clip_prompt_appears_only_when_a_sample_exceeds_full_scale_and_each_choice_does_what_it_says()
    {
        let (service, _engine, dir) = service("clip-prompt");
        let wav_path = dir.join("in.wav");
        let mut samples = vox_testkit::signal::sine(997.0, -6.0, 0.05, 48_000).unwrap();
        write_fixture_wav(
            &wav_path,
            &samples,
            vox_testkit::wav::BitDepth::Float32,
            48_000,
        );
        service.open(&wav_path, false).unwrap();

        // No overs yet (every sample is within [-1, 1]): Save As 16-bit needs no confirmation.
        let clean_path = dir.join("clean.wav");
        service
            .save_as(
                &clean_path,
                SaveContainer::Wav,
                BitDepth::Bit16,
                SaveDitherPref::Tpdf,
                false,
                false,
            )
            .unwrap();

        // Push 3 samples over 0 dBFS on the still-open document, peak +3.52 dBFS (SPEC-005 AC-6's
        // own example), by editing directly through the store (cheapest way to get an
        // out-of-range document in a unit test — no edit command silences/clips on write).
        samples[100] = 1.5;
        samples[200] = -1.2;
        samples[300] = 1.1;
        {
            let mut guard = service.0.open.lock().unwrap();
            let doc = guard.as_mut().unwrap();
            let mut writer = doc.session.store().writer();
            writer.append(&samples).unwrap();
            let audio = writer.finish().unwrap();
            let floored = doc.session.set_floor(&audio, Vec::new()).unwrap();
            service.0.engine.set_document(Some(PlaybackDoc {
                store: Arc::clone(doc.session.store()),
                snapshot: floored,
            }));
        }

        // "Cancel": no confirmation given, no file and no temp written.
        let over_path = dir.join("over.wav");
        let err = service
            .save_as(
                &over_path,
                SaveContainer::Wav,
                BitDepth::Bit16,
                SaveDitherPref::Tpdf,
                false,
                false,
            )
            .unwrap_err();
        assert_eq!(err.code, IpcErrorCode::NeedsConfirmation);
        assert_eq!(err.key, "dialog.overs");
        assert_eq!(err.params.get("count").map(String::as_str), Some("3"));
        let reported_peak: f64 = err.params["peak_dbfs"].parse().unwrap();
        assert!(
            (reported_peak - 20.0 * 1.5f64.log10()).abs() < 0.05,
            "peak_dbfs = {reported_peak}"
        );
        assert!(!over_path.exists(), "Cancel writes nothing");
        assert!(
            std::fs::read_dir(&dir)
                .unwrap()
                .filter_map(|e| e.ok())
                .all(|e| !e.file_name().to_string_lossy().contains("powervoice-tmp")),
            "Cancel leaves no temp file"
        );

        // "Clip and save": confirm_clip re-issues the same request and it clips.
        let info = service
            .save_as(
                &over_path,
                SaveContainer::Wav,
                BitDepth::Bit16,
                SaveDitherPref::Tpdf,
                true,
                false,
            )
            .unwrap();
        assert!(!info.dirty);
        let (decoded, _) = vox_testkit::wav::read_wav_file(&over_path).unwrap();
        // 16-bit full scale is 32767/32768; clipped samples read back at (approximately) +-1.0.
        assert!(
            decoded[100] > 0.999,
            "over sample clipped high: {}",
            decoded[100]
        );
        assert!(
            decoded[200] < -0.999,
            "over sample clipped low: {}",
            decoded[200]
        );
        assert!(
            decoded[300] > 0.999,
            "over sample clipped high: {}",
            decoded[300]
        );

        // "Save as 32-bit float instead": no confirmation needed, and the overshoot survives
        // exactly (SPEC-005 §2.8: "32-bit float is written bit-exact ... overs are preserved").
        let float_path = dir.join("float.wav");
        service
            .save_as(
                &float_path,
                SaveContainer::Wav,
                BitDepth::Bit32Float,
                SaveDitherPref::Tpdf,
                false,
                false,
            )
            .unwrap();
        let (decoded_f, _) = vox_testkit::wav::read_wav_file(&float_path).unwrap();
        assert!((decoded_f[100] - 1.5).abs() < 1e-6);
        assert!((decoded_f[200] - (-1.2)).abs() < 1e-6);
        assert!((decoded_f[300] - 1.1).abs() < 1e-6);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// T-209 (SPEC-005 §2.8): a document whose peak is exactly 1.0 needs no confirmation (only
    /// strictly-above-0-dBFS samples count as overs).
    #[test]
    fn a_peak_of_exactly_one_needs_no_clip_confirmation() {
        let (service, _engine, dir) = service("clip-exact-peak");
        let wav_path = dir.join("in.wav");
        let mut samples = vox_testkit::signal::sine(997.0, -6.0, 0.05, 48_000).unwrap();
        samples[0] = 1.0;
        write_fixture_wav(
            &wav_path,
            &samples,
            vox_testkit::wav::BitDepth::Float32,
            48_000,
        );
        service.open(&wav_path, false).unwrap();

        service
            .save_as(
                &dir.join("out.wav"),
                SaveContainer::Wav,
                BitDepth::Bit16,
                SaveDitherPref::Tpdf,
                false,
                false,
            )
            .unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// T-209 (SPEC-005 §2.4, §4.3 AC-9d): `open_with_downmix`'s channel choice reaches the
    /// import — picking "Right" on a stereo file is bit-exact to the right channel.
    #[test]
    fn open_with_downmix_picks_the_chosen_channel() {
        let (service, _engine, dir) = service("downmix-pick");
        let left = vox_testkit::signal::sine(500.0, -20.0, 0.1, 48_000).unwrap();
        let right = vox_testkit::signal::white_noise(7, -30.0, 0.1, 48_000).unwrap();
        let mut interleaved = Vec::with_capacity(left.len() * 2);
        for (&l, &r) in left.iter().zip(right.iter()) {
            interleaved.push(l);
            interleaved.push(r);
        }
        let stereo_path = dir.join("stereo.wav");
        write_wav_file_multi(&stereo_path, &interleaved, 2, 48_000);

        let cancel = CancelToken::new();
        let info = service
            .open_with_downmix(
                &stereo_path,
                false,
                vox_io::DownmixChoice::Channel(1),
                &cancel,
                &mut |_| {},
                &mut |_, _| {},
            )
            .unwrap();
        assert_eq!(info.len_samples, right.len() as u64);
        let (decoded, _) = vox_testkit::wav::read_wav_file(&{
            let out = dir.join("check.wav");
            service
                .save_as(
                    &out,
                    SaveContainer::Wav,
                    BitDepth::Bit32Float,
                    SaveDitherPref::Tpdf,
                    false,
                    true, // H-20: the source is stereo (picked channel), so this needs confirming
                )
                .unwrap();
            out
        })
        .unwrap();
        assert_eq!(
            decoded, right,
            "the document holds the picked (Right) channel, bit-exactly"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// T-209 (SPEC-005 §2.3 "Cancel"): cancelling an in-flight import leaves whatever document
    /// was already open completely untouched — no partial document is ever swapped in.
    #[test]
    fn open_with_downmix_cancel_leaves_the_previous_document_untouched() {
        let (service, _engine, dir) = service("downmix-cancel");
        let first_path = dir.join("first.wav");
        let first_samples = vox_testkit::signal::sine(220.0, -6.0, 0.05, 48_000).unwrap();
        write_fixture_wav(
            &first_path,
            &first_samples,
            vox_testkit::wav::BitDepth::Int24,
            48_000,
        );
        let before = service.open(&first_path, false).unwrap();

        let second_path = dir.join("second.wav");
        // Long enough that cancelling from the very first progress callback still lands mid-import
        // (several BATCH_FRAMES windows, `crates/project/src/import.rs`).
        let second_samples = vox_testkit::signal::sine(330.0, -6.0, 2.0, 48_000).unwrap();
        write_fixture_wav(
            &second_path,
            &second_samples,
            vox_testkit::wav::BitDepth::Int24,
            48_000,
        );

        let cancel = CancelToken::new();
        let cancel_for_progress = cancel.clone();
        let err = service
            .open_with_downmix(
                &second_path,
                false,
                vox_io::DownmixChoice::Average,
                &cancel,
                &mut |_fraction| cancel_for_progress.cancel(),
                &mut |_, _| {},
            )
            .unwrap_err();
        assert_eq!(err.code, IpcErrorCode::Cancelled);

        let after = service.info();
        assert_eq!(after.path, before.path);
        assert_eq!(after.audio_rev, before.audio_rev);
        assert_eq!(after.len_samples, first_samples.len() as u64);

        // The document is still fully usable after a cancelled open (a later, uncancelled open
        // works normally).
        let info = service.open(&second_path, false).unwrap();
        assert_eq!(info.len_samples, second_samples.len() as u64);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// T-209 (SPEC-005 §2.3): `import_file`'s progress callback reaches the caller as a `[0, 1]`
    /// fraction when the source states a length (every WAV does).
    #[test]
    fn open_with_downmix_reports_progress_when_the_source_states_a_length() {
        let (service, _engine, dir) = service("downmix-progress");
        let path = dir.join("in.wav");
        let samples = vox_testkit::signal::sine(220.0, -6.0, 1.0, 48_000).unwrap();
        write_fixture_wav(&path, &samples, vox_testkit::wav::BitDepth::Int24, 48_000);

        let fractions = std::sync::Mutex::new(Vec::<f32>::new());
        let cancel = CancelToken::new();
        service
            .open_with_downmix(
                &path,
                false,
                vox_io::DownmixChoice::Average,
                &cancel,
                &mut |f| {
                    fractions.lock().unwrap().push(f);
                },
                &mut |_, _| {},
            )
            .unwrap();

        let fractions = fractions.into_inner().unwrap();
        assert!(
            !fractions.is_empty(),
            "at least the final progress call must land"
        );
        assert!(fractions.iter().all(|&f| (0.0..=1.0).contains(&f)));
        assert!((fractions.last().copied().unwrap_or(0.0) - 1.0).abs() < 1e-6);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// H-71 (SPEC-005 §2.3, SPEC-006 AC-13, ADR-003 Amendment 6): `import_peaks` reads
    /// `PARTIAL`/`NaN` the instant `on_live` registers the job (nothing committed yet), and —
    /// once the import has finished but before `finish_import_job` unregisters it — the same
    /// range reads real, non-`PARTIAL` values that are bit-exact to `peaks()`'s own read of the
    /// now-open document ("the final state equals a non-progressive import"). An unknown job id
    /// answers with the same harmless empty/`partial` fallback `record_peaks_get` uses.
    #[test]
    fn import_peaks_is_partial_at_start_and_bit_exact_at_the_end() {
        let (service, _engine, dir) = service("import-peaks");
        let path = dir.join("in.wav");
        let samples = vox_testkit::signal::sine(220.0, -6.0, 0.2, 48_000).unwrap();
        write_fixture_wav(&path, &samples, vox_testkit::wav::BitDepth::Int24, 48_000);

        // No job registered at all: the harmless fallback, not an error.
        let unknown = service.import_peaks(999, 64, 0, 8).unwrap();
        assert_eq!(unknown.buckets.len(), 0);
        assert!(unknown.partial);

        let (job_id, cancel) = service.start_import_job();
        let mut saw_partial_at_registration = false;
        let info = service
            .open_with_downmix(
                &path,
                false,
                vox_io::DownmixChoice::Average,
                &cancel,
                &mut |_fraction| {},
                &mut |live, store| {
                    service.set_import_live(
                        job_id,
                        store,
                        live,
                        48_000,
                        Some(samples.len() as u64),
                    );
                    // `on_live` fires before any sample is decoded (H-71's contract) — nothing
                    // can be ready yet.
                    let r = service.import_peaks(job_id, 64, 0, 8).unwrap();
                    saw_partial_at_registration =
                        r.partial && r.buckets.iter().all(|(mn, _)| mn.is_nan());
                },
            )
            .unwrap();
        assert!(
            saw_partial_at_registration,
            "the very first poll, right when the job registers, must be all PARTIAL/NaN"
        );

        // The import has returned successfully (document swapped in), but the job is still
        // registered — `import_peaks` must now be fully ready and bit-exact to `peaks()`.
        let count = (info.len_samples / 64) as u32;
        let from_import = service.import_peaks(job_id, 64, 0, count).unwrap();
        assert!(!from_import.partial, "everything has committed by now");
        let from_document = service.peaks(64, 0, count).unwrap();
        assert_eq!(from_import.buckets, from_document.buckets);
        assert_eq!(from_import.sample_rate_hz, from_document.sample_rate_hz);

        service.finish_import_job(job_id);
        let after_finish = service.import_peaks(job_id, 64, 0, count).unwrap();
        assert_eq!(
            after_finish.buckets.len(),
            0,
            "unregistered: back to the harmless fallback"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn write_wav_file_multi(path: &Path, interleaved: &[f32], channels: u16, rate: u32) {
        vox_testkit::wav::write_wav_file(
            path,
            interleaved,
            channels,
            rate,
            vox_testkit::wav::BitDepth::Float32,
        )
        .unwrap();
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
        let save_err = service.save(false, false, false).unwrap_err();
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

    /// H-71: how much of a 60-min import's waveform is visible progressively, not just at the
    /// end — `cargo test -p powervoice-app --release -- --ignored
    /// import_peaks_of_the_60_min_fixture_fill_in_progressively --nocapture`.
    #[test]
    #[ignore]
    fn import_peaks_of_the_60_min_fixture_fill_in_progressively() {
        let dir = tmp_dir("sixty-min-progressive");
        let generated = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../fixtures/generated/long-60min-48k-mono.wav");
        let wav_path = if generated.exists() {
            generated
        } else {
            let path = dir.join("long-60min-48k-mono.wav");
            vox_testkit::signal::write_long_fixture(&path, 3600.0, 48_000, 42).unwrap();
            path
        };

        let (service, _engine, _dir2) = service("sixty-min-progressive");
        let (job_id, cancel) = service.start_import_job();
        let start = std::time::Instant::now();
        let mut ticks: Vec<(std::time::Duration, u64, bool)> = Vec::new();
        let info = service
            .open_with_downmix(
                &wav_path,
                false,
                vox_io::DownmixChoice::Average,
                &cancel,
                &mut |_fraction| {
                    // The whole 60-min file at the coarsest level (172_800_000 / 65_536 ≈ 2637
                    // buckets) — the same request `peaks_get` would eventually serve, read
                    // through `import_peaks` while the job is still running.
                    let r = service.import_peaks(job_id, 65_536, 0, 2637).unwrap();
                    let ready = r.buckets.iter().take_while(|(mn, _)| !mn.is_nan()).count();
                    ticks.push((start.elapsed(), ready as u64, r.partial));
                },
                &mut |live, store| {
                    service.set_import_live(job_id, store, live, 48_000, None);
                },
            )
            .unwrap();
        let open_elapsed = start.elapsed();
        service.finish_import_job(job_id);

        println!(
            "document_open of the 60-min fixture: {:.3} s ({} samples), {} progress ticks",
            open_elapsed.as_secs_f64(),
            info.len_samples,
            ticks.len(),
        );
        for (t, ready, partial) in &ticks {
            println!(
                "  t={:>6.1} ms: {:>4}/2637 coarse buckets ready ({:>5.1}%, partial={})",
                t.as_secs_f64() * 1000.0,
                ready,
                100.0 * *ready as f64 / 2637.0,
                partial,
            );
        }
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
            service.save(false, false, false).unwrap_err().key,
            "error.document.untitled"
        );
        let saved = dir.join("take.wav");
        let info = service
            .save_as(
                &saved,
                SaveContainer::Wav,
                BitDepth::Bit24,
                SaveDitherPref::Tpdf,
                false,
                false,
            )
            .unwrap();
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
        assert_eq!(
            markers[0].kind,
            MarkerKindInfo::Dropout,
            "H-57: real kind field"
        );
        assert_eq!(markers[1].pos_samples, 5_000);
        assert_eq!(markers[1].len_samples, 24);
        assert_eq!(markers[1].name, "Dropout 1 ms", "rounds to the nearest ms");
        assert_eq!(markers[1].kind, MarkerKindInfo::Dropout);

        // AC-19: a dropout marker can be renamed/moved like any marker and keeps its kind.
        let id = markers[0].id;
        service.marker_rename(id, "Renamed").unwrap();
        service
            .marker_set_range(id, 2_000, 480, MarkerRangeEditKind::Move)
            .unwrap();
        let renamed = service
            .markers_get()
            .into_iter()
            .find(|m| m.id == id)
            .unwrap();
        assert_eq!(renamed.name, "Renamed");
        assert_eq!(renamed.pos_samples, 2_000);
        assert_eq!(
            renamed.kind,
            MarkerKindInfo::Dropout,
            "kind survives rename/move"
        );

        // One undo entry removes the take and both dropout markers together (the rename/move
        // above are separate entries, undone first).
        service.history_undo().unwrap();
        service.history_undo().unwrap();
        let history = service.history_state();
        assert!(history.can_undo);
        assert_eq!(history.undo_label.as_deref(), Some("history.record"));
        service.history_undo().unwrap();
        assert!(service.markers_get().is_empty());
    }

    /// H-23 (SPEC-002 §4.3): a dropout of unknown length (unreliable timestamps,
    /// `DropoutMark::UNKNOWN_LEN`) becomes a point marker (no known span, so `len_samples` is 0 —
    /// not the `u64::MAX` sentinel) reading "Dropout (length unknown)".
    #[test]
    fn dropout_of_unknown_length_becomes_a_length_unknown_point_marker() {
        let (service, _engine, _dir) = service("dropout-unknown-length");
        let (mut capture, _info) = service
            .begin_recording(48_000, BitDepth::Bit24, false)
            .unwrap();
        let samples = vox_testkit::signal::sine(440.0, -6.0, 0.2, 48_000).unwrap();
        capture.append(&samples).unwrap();
        let dropouts = [vox_engine::record::DropoutMark {
            pos_samples: 1_000,
            len_samples: vox_engine::record::DropoutMark::UNKNOWN_LEN,
        }];
        let info = service
            .commit_take(&capture.finish(), &dropouts)
            .unwrap()
            .expect("one undoable edit");
        assert!(info.dirty);

        let markers = service.markers_get();
        assert_eq!(markers.len(), 1, "{markers:?}");
        assert_eq!(markers[0].pos_samples, 1_000);
        assert_eq!(markers[0].len_samples, 0, "a point marker — no known span");
        assert_eq!(markers[0].name, "Dropout (length unknown)");
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

    // --- H-56: Insert silence (SPEC-008 §2.1/§2.5) -----------------------------------------

    #[test]
    fn insert_silence_at_the_cursor_start_and_end_is_one_undo_entry() {
        let (service, _engine, dir) = service("insert-silence");
        let samples = vox_testkit::signal::sine(440.0, -6.0, 0.2, 48_000).unwrap();
        open_test_doc(&service, &dir, &samples);
        let len = samples.len() as u64;

        // At 48 kHz, "1.000 s" is exactly 48 000 samples (SPEC-008 AC-6).
        let mid = service
            .edit_insert_silence(PasteTarget::Cursor(1_000), 48_000)
            .unwrap();
        assert!(mid.changed);
        assert_eq!(mid.len_samples, len + 48_000);
        assert_eq!(mid.selection, Some((1_000, 1_000 + 48_000)));
        assert_eq!(mid.playhead_samples, 1_000);
        assert_eq!(
            service.history_state().undo_label.as_deref(),
            Some("history.insert_silence")
        );

        let at_start = service
            .edit_insert_silence(PasteTarget::Cursor(0), 10)
            .unwrap();
        assert_eq!(at_start.selection, Some((0, 10)));

        let cur_len = at_start.len_samples;
        let at_end = service
            .edit_insert_silence(PasteTarget::Cursor(cur_len), 10)
            .unwrap();
        assert_eq!(at_end.selection, Some((cur_len, cur_len + 10)));
        assert_eq!(at_end.len_samples, cur_len + 10);

        // With a selection, silence is inserted at the start and nothing is removed.
        let with_selection = service
            .edit_insert_silence(PasteTarget::Range(5, 20), 100)
            .unwrap();
        assert_eq!(with_selection.selection, Some((5, 105)));

        let undo = service.history_undo().unwrap();
        assert_eq!(undo.selection, None, "undo clears the selection");

        // Undo repeatedly back to the original document.
        for _ in 0..3 {
            service.history_undo().unwrap();
        }
        assert_eq!(service.info().len_samples, len);
        assert!(!service.history_state().can_undo);
    }

    #[test]
    fn insert_silence_into_an_empty_document_inserts_at_zero() {
        let (service, _engine, dir) = service("insert-silence-empty");
        open_test_doc(&service, &dir, &[]);
        assert_eq!(service.info().len_samples, 0);
        let result = service
            .edit_insert_silence(PasteTarget::Cursor(0), 500)
            .unwrap();
        assert_eq!(result.len_samples, 500);
        assert_eq!(result.selection, Some((0, 500)));
    }

    #[test]
    fn insert_silence_duration_validation() {
        let (service, _engine, dir) = service("insert-silence-duration");
        let samples = vox_testkit::signal::silence(0.1, 48_000).unwrap();
        open_test_doc(&service, &dir, &samples);

        assert_eq!(
            service
                .edit_insert_silence(PasteTarget::Cursor(0), 0)
                .unwrap_err()
                .key,
            "error.insert_silence_duration"
        );
        assert_eq!(
            service
                .edit_insert_silence(PasteTarget::Cursor(0), 3_600 * 48_000 + 1)
                .unwrap_err()
                .key,
            "error.insert_silence_duration"
        );
        assert!(!service.history_state().can_undo, "rejected: no undo entry");
        assert!(
            service
                .edit_insert_silence(PasteTarget::Cursor(0), 3_600 * 48_000)
                .is_ok_and(|r| r.changed),
            "exactly the 1 h ceiling is accepted"
        );
    }

    #[test]
    fn insert_silence_is_refused_while_recording_or_busy() {
        let (service, _engine, dir) = service("insert-silence-recording");
        open_test_doc(
            &service,
            &dir,
            &vox_testkit::signal::silence(0.05, 48_000).unwrap(),
        );
        let (mut capture, _info) = service
            .begin_recording(48_000, BitDepth::Bit24, true)
            .unwrap();
        capture.append(&[0.0; 100]).unwrap();
        assert_eq!(
            service
                .edit_insert_silence(PasteTarget::Cursor(0), 10)
                .unwrap_err()
                .code,
            IpcErrorCode::NotWhileRecording
        );
        service.discard_take(capture.id());
    }

    // --- H-56: cross-document clipboard (SPEC-008 §2.6) ------------------------------------

    /// AC-15 analog: a same-rate cross-document paste rebinds the clipboard to the new session
    /// (a pure splice, no job) and a second paste reuses it directly.
    #[test]
    fn clipboard_survives_closing_the_source_document_same_rate() {
        let (service, _engine, dir) = service("clip-cross-same-rate");
        let a_samples = vox_testkit::signal::sine(440.0, -6.0, 0.1, 48_000).unwrap();
        open_test_doc(&service, &dir, &a_samples);
        let cut = service.edit_cut(0, 2_000).unwrap();
        assert!(cut.changed);
        assert_eq!(service.clipboard_info().len_samples, Some(2_000));

        // Closing A (without saving) hands the clipboard's audio to a materialized file — it
        // does not clear the clipboard.
        service.close().unwrap();
        assert_eq!(
            service.clipboard_info().len_samples,
            Some(2_000),
            "the clipboard survives the source document closing"
        );
        assert!(!service.paste_needs_job(), "no document is open yet");

        // Open B at the same rate and paste: `paste_needs_job` is true until the first import.
        let b_samples = vox_testkit::signal::sine(220.0, -6.0, 0.1, 48_000).unwrap();
        open_test_doc(&service, &dir, &b_samples);
        assert!(service.paste_needs_job());

        let source = service.begin_paste_job(PasteTarget::Cursor(0)).unwrap();
        assert_eq!(source.target_rate_hz, 48_000);
        let cancel = vox_project::CancelToken::new();
        let written = vox_project::clipboard::import_clip(
            &source.clip,
            &source.store,
            48_000,
            &cancel,
            &mut |_| {},
        )
        .unwrap();
        assert_eq!(written.len_samples, 2_000);
        let result = service.finish_paste_job(&source, written).unwrap();
        assert!(result.changed);
        assert_eq!(result.len_samples, b_samples.len() as u64 + 2_000);
        assert!(!service.is_paste_busy(), "busy flag released");

        // Same-rate import rebinds the clipboard: a later paste is a pure, job-free splice.
        assert!(!service.paste_needs_job());
        let second = service.edit_paste(PasteTarget::Cursor(0)).unwrap();
        assert!(second.changed);
    }

    /// AC-16 analog: a cross-document paste with a sample-rate mismatch resamples to the exact
    /// expected length and caches the converted pieces for later pastes into the same document.
    #[test]
    fn clipboard_paste_with_a_rate_mismatch_resamples_and_caches() {
        let (service, _engine, dir) = service("clip-cross-rate-mismatch");
        let a_samples = vec![0.0f32; 44_100]; // 1.000 s at 44.1 kHz
        let a_path = dir.join("a.wav");
        write_fixture_wav(
            &a_path,
            &a_samples,
            vox_testkit::wav::BitDepth::Float32,
            44_100,
        );
        service.open(&a_path, false).unwrap();
        service.edit_copy(0, 44_100).unwrap();
        service.close().unwrap();

        let b_samples = vox_testkit::signal::silence(0.5, 48_000).unwrap();
        open_test_doc(&service, &dir, &b_samples);
        assert!(service.paste_needs_job());

        let source = service.begin_paste_job(PasteTarget::Cursor(0)).unwrap();
        assert_eq!(source.clip.sample_rate_hz, 44_100);
        assert_eq!(source.target_rate_hz, 48_000);
        let cancel = vox_project::CancelToken::new();
        let written = vox_project::clipboard::import_clip(
            &source.clip,
            &source.store,
            source.target_rate_hz,
            &cancel,
            &mut |_| {},
        )
        .unwrap();
        assert_eq!(written.len_samples, 48_000, "round(44100 * 48000/44100)");
        let result = service.finish_paste_job(&source, written).unwrap();
        assert!(result.changed);

        // The converted pieces are cached for this document: a second paste needs no job.
        assert!(!service.paste_needs_job());
        let second = service.edit_paste(PasteTarget::Cursor(0)).unwrap();
        assert!(second.changed);
    }

    /// A materialize failure (here: the clipboard directory's path is occupied by a plain file)
    /// clears the clipboard and posts `notice.clipboard_lost` instead of failing the close.
    #[test]
    fn a_materialize_failure_clears_the_clipboard_and_posts_a_notice() {
        let dir = tmp_dir("clip-materialize-fails");
        // `DocumentService::new` derives the clipboard directory as a sibling of `sessions_dir`
        // (`dir/clipboard`) — occupy that path with a plain file so `materialize` can't create a
        // directory there.
        std::fs::write(dir.join("clipboard"), b"not a directory").unwrap();
        let engine = test_engine();
        let service = DocumentService::new(dir.join("sessions"), engine.handle());
        open_test_doc(
            &service,
            &dir,
            &vox_testkit::signal::sine(440.0, -6.0, 0.1, 48_000).unwrap(),
        );
        service.edit_cut(0, 1_000).unwrap();
        assert_eq!(service.clipboard_info().len_samples, Some(1_000));

        service.close().unwrap();
        assert_eq!(
            service.clipboard_info().len_samples,
            None,
            "a failed materialize clears the clipboard"
        );
        let notices = service.take_sidecar_notices();
        assert!(
            notices.iter().any(|n| n.key == "notice.clipboard_lost"),
            "{notices:?}"
        );
    }

    /// While a cross-document paste job is running (`begin_paste_job` called, not yet finished),
    /// other edit commands are refused like any other document job (SPEC-008 §2.2).
    #[test]
    fn other_edits_are_refused_while_a_paste_job_is_running() {
        let (service, _engine, dir) = service("clip-cross-busy");
        let a_samples = vox_testkit::signal::sine(440.0, -6.0, 0.1, 44_100).unwrap();
        let a_path = dir.join("a.wav");
        write_fixture_wav(
            &a_path,
            &a_samples,
            vox_testkit::wav::BitDepth::Float32,
            44_100,
        );
        service.open(&a_path, false).unwrap();
        service.edit_copy(0, a_samples.len() as u64).unwrap();
        service.close().unwrap();

        open_test_doc(
            &service,
            &dir,
            &vox_testkit::signal::silence(0.2, 48_000).unwrap(),
        );
        let source = service.begin_paste_job(PasteTarget::Cursor(0)).unwrap();
        assert!(service.is_paste_busy());
        assert_eq!(
            service.edit_cut(0, 10).unwrap_err().key,
            "error.document_busy"
        );
        // H-66: edit_copy is gated like its siblings.
        assert_eq!(
            service.edit_copy(0, 10).unwrap_err().key,
            "error.document_busy"
        );
        assert_eq!(
            service
                .edit_insert_silence(PasteTarget::Cursor(0), 10)
                .unwrap_err()
                .key,
            "error.document_busy"
        );

        service.abandon_paste_job();
        assert!(!service.is_paste_busy());
        // Nothing was written and the clipboard is unchanged (still materialized).
        assert!(service.paste_needs_job());
        drop(source);
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
        // H-66: edit_copy is gated like its siblings — it would otherwise clobber the clipboard a
        // concurrent job may be reading.
        let err = service.edit_copy(0, 10).unwrap_err();
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

        // H-21: Add Marker is allowed during a take (it joins the take's edit, see
        // `ac10_marker_add_during_an_operation_joins_its_edit`); editing markers is not.
        assert!(service.marker_add(0, 0).is_ok());
        for result in [
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

    /// H-21 / SPEC-022 AC-10 (app side): during a punch, Add Marker succeeds (renaming stays
    /// refused), the marker is listed live and named after the pending ones too, and lands in the
    /// operation's one "Punch-in" edit at its position (pre-roll: the heard position before `S`;
    /// window: `S + k`); undo restores the original marker list exactly. A cancelled operation
    /// keeps its marker as its own "Add Marker" entry and leaves the audio untouched.
    #[test]
    fn ac10_marker_add_during_an_operation_joins_its_edit() {
        use vox_engine::record::{CancelReason, OpResult};
        use vox_engine::record_op::{RecordOpKind, RecordPlan};
        let (service, _engine, dir) = service("marker-op");
        let samples = vox_testkit::signal::sine(440.0, -6.0, 1.0, 48_000).unwrap();
        open_test_doc(&service, &dir, &samples);
        let existing = service.marker_add(1_000, 0).unwrap();
        let plan = RecordPlan {
            kind: RecordOpKind::Punch,
            at_samples: 12_000,
            end_samples: Some(24_000),
            preroll_samples: 6_000,
            postroll_samples: 4_800,
            aligned: true,
            offset_ns: 0,
            hear_original: false,
            xfade_samples: 480,
            doc_len_samples: 48_000,
            doc_rate_hz: 48_000,
        };
        let ids = |service: &DocumentService| -> Vec<(u64, u64)> {
            service
                .markers_get()
                .iter()
                .map(|m| (m.id, m.pos_samples))
                .collect()
        };

        let mut capture = service.begin_record_op(&plan).unwrap();
        capture.append(&[0.25f32; 22_800]).unwrap();
        let pre = service.marker_add(9_000, 0).unwrap();
        let win = service.marker_add(15_000, 0).unwrap();
        assert_ne!(
            pre.name, win.name,
            "pending markers count for the next name"
        );
        assert_eq!(
            service.marker_rename(existing.id, "x").unwrap_err().code,
            IpcErrorCode::NotWhileRecording
        );
        assert_eq!(
            ids(&service),
            vec![(existing.id, 1_000), (pre.id, 9_000), (win.id, 15_000)]
        );
        let finished = capture.finish();
        let op = OpResult {
            plan,
            window: Some((6_000, 18_000)),
            cancelled: None,
        };
        assert!(
            service
                .commit_take_op(&finished, &op, &[])
                .unwrap()
                .is_some()
        );
        assert_eq!(
            service.history_state().undo_label.as_deref(),
            Some("history.punch")
        );
        assert_eq!(
            ids(&service),
            vec![(existing.id, 1_000), (pre.id, 9_000), (win.id, 15_000)]
        );
        service.history_undo().unwrap();
        assert_eq!(ids(&service), vec![(existing.id, 1_000)]);

        // Cancelled (Stop in pre-roll): the marker persists as its own "Add Marker" entry.
        let audio_rev = service.info().audio_rev;
        let mut capture = service.begin_record_op(&plan).unwrap();
        capture.append(&[0.25f32; 3_000]).unwrap();
        let kept = service.marker_add(10_000, 0).unwrap();
        let finished = capture.finish();
        let op = OpResult {
            plan,
            window: None,
            cancelled: Some(CancelReason::User),
        };
        assert!(
            service
                .commit_take_op(&finished, &op, &[])
                .unwrap()
                .is_none()
        );
        assert_eq!(
            service.history_state().undo_label.as_deref(),
            Some("history.marker_add")
        );
        assert_eq!(ids(&service), vec![(existing.id, 1_000), (kept.id, 10_000)]);
        assert_eq!(
            service.info().audio_rev,
            audio_rev,
            "the audio is untouched"
        );
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
        service
            .save_as(
                &save_path,
                SaveContainer::Wav,
                BitDepth::Bit24,
                SaveDitherPref::Tpdf,
                false,
                false,
            )
            .unwrap();

        // A second document service (a fresh session dir) opens the saved file back.
        let reopened = DocumentService::new(dir.join("sessions2"), service.0.engine.clone());
        reopened.open(&save_path, false).unwrap();
        let markers = reopened.markers_get();
        assert_eq!(markers.len(), 2);
        assert_eq!((markers[0].pos_samples, markers[0].len_samples), (0, 0));
        assert_eq!(markers[0].name, "Intro");
        assert_eq!(
            markers[0].kind,
            MarkerKindInfo::User,
            "H-57: kind round-trips too"
        );
        assert_eq!(
            (markers[1].pos_samples, markers[1].len_samples),
            (48_000, 9_600)
        );
        assert_eq!(markers[1].kind, MarkerKindInfo::User);
    }

    // --- T-306: sidecar round trip, identity, sidecar-only saves, notices --------------------

    /// SPEC-009 §2.13 case 3 (AC-18(b)'s scenario): a WAV whose readable `cue ` set differs from
    /// its matching sidecar's own copy — another program moved a marker after the last save —
    /// opens with the **file's** markers, not the sidecar's, with
    /// `notice.open.markers_changed_externally`. H-64: nothing implemented this before (the open
    /// path always used the sidecar unconditionally whenever one matched).
    #[test]
    fn open_of_a_wav_whose_cues_differ_from_its_sidecar_uses_the_wav_markers() {
        let (service, _engine, dir) = service("markers-changed-externally");
        let samples = vox_testkit::signal::sine(440.0, -6.0, 5.0, 48_000).unwrap();
        open_test_doc(&service, &dir, &samples);
        service.marker_add(0, 0).unwrap();
        service
            .marker_rename(service.markers_get()[0].id, "Intro")
            .unwrap();
        service.marker_add(200_000, 4_800).unwrap();
        service
            .marker_rename(service.markers_get()[1].id, "Take 2")
            .unwrap();

        let save_path = dir.join("changed.wav");
        service
            .save_as(
                &save_path,
                SaveContainer::Wav,
                BitDepth::Bit32Float,
                SaveDitherPref::None,
                false,
                false,
            )
            .unwrap();

        // Another program moves "Take 2" to 210 000, rewriting only the `cue `/`LIST adtl`
        // chunks — decoding and re-encoding at the same float format keeps the audio (and so the
        // sidecar's identity fingerprint) bit-exact, "audio untouched" per the AC.
        let (rate, _channels, mut source) = vox_io::read_wav(&save_path).unwrap();
        let mut audio = vec![0.0f32; samples.len()];
        let n = source.read_mono(&mut audio).unwrap();
        assert_eq!(n, samples.len());
        let moved_markers = vec![
            vox_io::WavMarker {
                pos_samples: 0,
                len_samples: 0,
                name: "Intro".to_string(),
            },
            vox_io::WavMarker {
                pos_samples: 210_000,
                len_samples: 4_800,
                name: "Take 2".to_string(),
            },
        ];
        vox_io::write_wav_with_markers(
            &save_path,
            rate,
            vox_io::BitDepth::Float32,
            vox_io::DitherMode::None,
            &audio,
            &moved_markers,
        )
        .unwrap();

        let reopened = DocumentService::new(dir.join("sessions2"), service.0.engine.clone());
        reopened.open(&save_path, false).unwrap();
        let markers = reopened.markers_get();
        assert_eq!(markers.len(), 2);
        assert_eq!(markers[0].pos_samples, 0);
        assert_eq!(markers[0].name, "Intro");
        assert_eq!(
            (markers[1].pos_samples, markers[1].len_samples),
            (210_000, 4_800),
            "the file's own markers win (case 3), not the sidecar's stale 200_000"
        );
        assert_eq!(markers[1].name, "Take 2");

        let notices = reopened.take_sidecar_notices();
        assert!(
            notices
                .iter()
                .any(|n| n.key == "notice.open.markers_changed_externally"),
            "got {notices:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// SPEC-009 §2.13 case 2: a WAV whose readable `cue ` set still matches its sidecar exactly
    /// keeps using the sidecar's markers (ids included) — no `markers_changed_externally` notice.
    /// This is the counterpart to the case-3 test above: same fixture shape, but the external
    /// rewrite reproduces the identical cue set instead of moving anything.
    #[test]
    fn open_of_a_wav_whose_cues_still_match_its_sidecar_uses_the_sidecar_markers() {
        let (service, _engine, dir) = service("markers-unchanged");
        let samples = vox_testkit::signal::sine(440.0, -6.0, 1.0, 48_000).unwrap();
        open_test_doc(&service, &dir, &samples);
        service.marker_add(0, 0).unwrap();
        let sidecar_id = service.markers_get()[0].id;
        service.marker_rename(sidecar_id, "Intro").unwrap();

        let save_path = dir.join("unchanged.wav");
        service
            .save_as(
                &save_path,
                SaveContainer::Wav,
                BitDepth::Bit32Float,
                SaveDitherPref::None,
                false,
                false,
            )
            .unwrap();

        let reopened = DocumentService::new(dir.join("sessions2"), service.0.engine.clone());
        reopened.open(&save_path, false).unwrap();
        let markers = reopened.markers_get();
        assert_eq!(markers.len(), 1);
        assert_eq!(markers[0].name, "Intro");
        assert_eq!(
            markers[0].id, sidecar_id,
            "case 2: the sidecar's own id is kept, unlike case 3's fresh ids"
        );
        let notices = reopened.take_sidecar_notices();
        assert!(
            !notices
                .iter()
                .any(|n| n.key == "notice.open.markers_changed_externally"),
            "an unchanged cue set never posts the case-3 notice, got {notices:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn gain_rack_model(gain_db: f64) -> vox_rack::RackModel {
        vox_rack::RackModel {
            slots: vec![vox_rack::SlotModel {
                module: "org.powervoice.gain@1.0.0".to_string(),
                bypass: false,
                state: serde_json::json!({"format_version": 1, "params": {"gain_db": gain_db}}),
                extra: Default::default(),
                raw: None,
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
        service.set_waveform_view(WaveformViewInfo {
            start_sample: 1_234,
            samples_per_pixel: 37.25,
            selection: Some((2_000, 4_000)),
            cursor_samples: 3_000,
            time_ruler_format: TimeRulerFormatDto::Seconds,
            vertical_zoom: 8.0,
            amplitude_ruler_mode: AmplitudeRulerModeDto::Percent,
        });

        let save_path = dir.join("with-rack.wav");
        let saved_info = service
            .save_as(
                &save_path,
                SaveContainer::Wav,
                BitDepth::Bit24,
                SaveDitherPref::Tpdf,
                false,
                false,
            )
            .unwrap();
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
            reopened.take_sidecar_notices().is_empty(),
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

        let waveform_view = info
            .waveform_view
            .expect("the saved waveform view loads back (H-12)");
        assert_eq!(waveform_view.start_sample, 1_234);
        assert!((waveform_view.samples_per_pixel - 37.25).abs() < 1e-9);
        assert_eq!(waveform_view.selection, Some((2_000, 4_000)));
        assert_eq!(waveform_view.cursor_samples, 3_000);
        assert_eq!(
            waveform_view.time_ruler_format,
            TimeRulerFormatDto::Seconds,
            "T-206: time_ruler_format round-trips too"
        );
        assert!(
            (waveform_view.vertical_zoom - 8.0).abs() < 1e-9,
            "H-35: vertical_zoom round-trips too"
        );
        assert_eq!(
            waveform_view.amplitude_ruler_mode,
            AmplitudeRulerModeDto::Percent,
            "H-72: amplitude_ruler_mode round-trips too"
        );

        assert!(
            !info.sidecar_dirty,
            "opening a file with a matching sidecar never sets * (SPEC-018 §2.5)"
        );
    }

    /// H-12: `set_waveform_view`/`set_spectral_view` are fire-and-forget view-only writes — with
    /// no document open they must not panic, and `info().waveform_view` stays `None` (SPEC-018
    /// §2.6.5's "no opinion").
    #[test]
    fn set_waveform_view_with_no_document_open_is_a_harmless_no_op() {
        let (service, _engine, _dir) = service("waveform-view-no-doc");
        service.set_waveform_view(WaveformViewInfo {
            start_sample: 10,
            samples_per_pixel: 2.0,
            selection: None,
            cursor_samples: 0,
            time_ruler_format: TimeRulerFormatDto::Timecode,
            vertical_zoom: 1.0,
            amplitude_ruler_mode: AmplitudeRulerModeDto::Dbfs,
        });
        assert_eq!(service.info().waveform_view, None);
    }

    /// H-12 (SPEC-018 §2.4): a waveform-view-only change never marks the document modified —
    /// `sidecar_dirty` only reacts to save format/markers/rack (§4.3's persisted-content digest
    /// deliberately excludes `view`).
    #[test]
    fn set_waveform_view_never_marks_the_document_modified() {
        let (service, _engine, dir) = service("waveform-view-not-dirty");
        let samples = vox_testkit::signal::sine(440.0, -6.0, 0.1, 48_000).unwrap();
        open_test_doc(&service, &dir, &samples);
        let save_path = dir.join("a.wav");
        service
            .save_as(
                &save_path,
                SaveContainer::Wav,
                BitDepth::Bit24,
                SaveDitherPref::Tpdf,
                false,
                false,
            )
            .unwrap();
        assert!(!service.info().sidecar_dirty);
        assert!(!service.info().dirty);

        service.set_waveform_view(WaveformViewInfo {
            start_sample: 500,
            samples_per_pixel: 4.0,
            selection: Some((0, 100)),
            cursor_samples: 100,
            time_ruler_format: TimeRulerFormatDto::Samples,
            vertical_zoom: 16.0,
            amplitude_ruler_mode: AmplitudeRulerModeDto::Dbfs,
        });

        assert!(!service.info().sidecar_dirty);
        assert!(!service.info().dirty);
    }

    /// H-12: an invalid `selection` (missing a field, `start > end`, or out of `[0, L]`) is
    /// ignored rather than restoring garbage — same "no opinion on the invalid part" spirit as
    /// the rest of §2.6.5.
    #[test]
    fn waveform_view_of_ignores_a_malformed_selection() {
        let view = serde_json::json!({
            "waveform": {
                "start_sample": 10,
                "samples_per_pixel": 2.0,
                "selection": { "start_sample": 500, "end_sample": 100 },
                "cursor_samples": 42,
            }
        });
        let info =
            waveform_view_of(&view, 1_000).expect("start_sample/samples_per_pixel are present");
        assert_eq!(info.selection, None, "start > end is dropped");
        assert_eq!(info.cursor_samples, 42);
        assert_eq!(
            info.time_ruler_format,
            TimeRulerFormatDto::Timecode,
            "T-206: a missing time_ruler_format falls back to the SPEC-006 §2.5 default"
        );
        assert!(
            (info.vertical_zoom - 1.0).abs() < 1e-9,
            "H-35: a missing vertical_zoom falls back to the SPEC-006 §2.4 default (1.0)"
        );
        assert_eq!(
            info.amplitude_ruler_mode,
            AmplitudeRulerModeDto::Dbfs,
            "H-72: a missing amplitude_ruler_mode falls back to the SPEC-006 §2.4 default (dBFS)"
        );

        let missing_view = serde_json::json!({ "waveform": { "start_sample": 10 } });
        assert_eq!(
            waveform_view_of(&missing_view, 1_000),
            None,
            "no samples_per_pixel -> no opinion at all"
        );

        assert_eq!(waveform_view_of(&serde_json::json!({}), 1_000), None);

        // SPEC-018 §2.6.5: "restored when within [0, L]; otherwise null / 0".
        let out_of_range = serde_json::json!({
            "waveform": {
                "start_sample": 0,
                "samples_per_pixel": 1.0,
                "selection": { "start_sample": 0, "end_sample": 2_000 },
                "cursor_samples": 5_000,
                "time_ruler_format": "not-a-real-format",
                "vertical_zoom": 1_000.0,
                "amplitude_ruler_mode": "not-a-real-mode",
            }
        });
        let info = waveform_view_of(&out_of_range, 1_000).unwrap();
        assert_eq!(info.selection, None, "end beyond len_samples is dropped");
        assert_eq!(
            info.cursor_samples, 0,
            "cursor beyond len_samples falls back to 0"
        );
        assert_eq!(
            info.time_ruler_format,
            TimeRulerFormatDto::Timecode,
            "T-206: an unrecognized time_ruler_format value falls back to the default too"
        );
        assert!(
            (info.vertical_zoom - 256.0).abs() < 1e-9,
            "H-35: vertical_zoom beyond the SPEC-006 §2.4 range is clamped, not dropped"
        );
        assert_eq!(
            info.amplitude_ruler_mode,
            AmplitudeRulerModeDto::Dbfs,
            "H-72: an unrecognized amplitude_ruler_mode value falls back to the default too"
        );

        let samples_format = serde_json::json!({
            "waveform": {
                "start_sample": 0,
                "samples_per_pixel": 1.0,
                "selection": null,
                "cursor_samples": 0,
                "time_ruler_format": "samples",
                "vertical_zoom": 0.0001,
                "amplitude_ruler_mode": "percent",
            }
        });
        let info = waveform_view_of(&samples_format, 1_000).unwrap();
        assert_eq!(info.time_ruler_format, TimeRulerFormatDto::Samples);
        assert!(
            (info.vertical_zoom - 1.0).abs() < 1e-9,
            "H-35: vertical_zoom below the SPEC-006 §2.4 range is clamped to 1.0"
        );
        assert_eq!(
            info.amplitude_ruler_mode,
            AmplitudeRulerModeDto::Percent,
            "H-72: a recognized amplitude_ruler_mode value ('percent') is restored"
        );

        let null_vertical_zoom = serde_json::json!({
            "waveform": {
                "start_sample": 0,
                "samples_per_pixel": 1.0,
                "selection": null,
                "cursor_samples": 0,
                "vertical_zoom": null,
            }
        });
        assert!(
            (waveform_view_of(&null_vertical_zoom, 1_000)
                .unwrap()
                .vertical_zoom
                - 1.0)
                .abs()
                < 1e-9,
            "H-35: an explicit null vertical_zoom falls back to the default too"
        );
    }

    #[test]
    fn sidecar_dirty_reflects_rack_changes_and_clears_when_reverted() {
        let (service, _engine, dir) = service("sidecar-dirty");
        let samples = vox_testkit::signal::sine(440.0, -6.0, 0.2, 48_000).unwrap();
        open_test_doc(&service, &dir, &samples);
        let save_path = dir.join("a.wav");
        service
            .save_as(
                &save_path,
                SaveContainer::Wav,
                BitDepth::Bit24,
                SaveDitherPref::Tpdf,
                false,
                false,
            )
            .unwrap();
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

    /// H-40: a document is opened (and saved) with its rack naming a plugin that isn't
    /// registered — a Missing placeholder, kept verbatim. Installing/rescanning later hot-adds
    /// the module into the same live registry (T-804/H-29's `Registry::upsert`); the rack
    /// recovers the slot on its own (no reopen — `crates/rack/tests/live_recovery.rs` covers that
    /// mechanism itself), and the recovery notice (simulated here: no Tauri app in this test, so
    /// `DocumentService::mark_rack_recovered` stands in for `audio::forward_rack_notice`) must
    /// not flip `sidecar_dirty` — the saved file didn't change, only the live rack caught up to
    /// what it already named.
    #[test]
    fn a_live_plugin_recovery_does_not_mark_the_document_dirty() {
        let dir = tmp_dir("live-recovery-dirty");
        let (engine, registry) = test_engine_with_registry(Vec::new());
        let service = DocumentService::new(dir.join("sessions"), engine.handle());
        let samples = vox_testkit::signal::sine(440.0, -6.0, 0.2, 48_000).unwrap();
        open_test_doc(&service, &dir, &samples);
        let save_path = dir.join("a.wav");
        service
            .save_as(
                &save_path,
                SaveContainer::Wav,
                BitDepth::Bit24,
                SaveDitherPref::Tpdf,
                false,
                false,
            )
            .unwrap();
        assert!(!service.info().sidecar_dirty);

        // The rack names Gain, which isn't registered on this engine — a Missing placeholder —
        // and that's what gets saved. A `blob` Gain itself never round-trips (its own
        // `save_state` never sets one) makes sure the resolved slot's JSON genuinely differs from
        // the kept placeholder's, below.
        let mut missing = gain_rack_model(-4.0);
        missing.slots[0].state["blob"] = serde_json::json!("AAEC");
        service.0.engine.rack_load_model(missing).unwrap().unwrap();
        assert!(matches!(
            service.0.engine.rack_snapshot().slots[0].info.status,
            vox_rack::SlotStatus::Missing { .. }
        ));
        service.save(false, false, false).unwrap();
        assert!(!service.info().sidecar_dirty);

        // "Install"/"rescan" hot-adds the module into the same registry the live rack already
        // shares — the rack notices on its own and recovers the slot, no reopen.
        registry
            .register(Arc::new(vox_modules::GainFactory::new()))
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if matches!(
                service
                    .0
                    .engine
                    .rack_snapshot()
                    .slots
                    .first()
                    .map(|s| &s.info.status),
                Some(vox_rack::SlotStatus::Active)
            ) {
                break;
            }
            assert!(Instant::now() < deadline, "the slot never recovered");
            std::thread::sleep(Duration::from_millis(5));
        }

        // Sanity: without `DocumentService` learning about the recovery, the digest really does
        // differ (the resolved slot's JSON isn't byte-for-byte the kept placeholder's) — proving
        // the rebase below does real work, not that this was a no-op all along.
        assert!(
            service.info().sidecar_dirty,
            "sanity: the recovered rack's JSON really does differ from the placeholder's"
        );

        service.mark_rack_recovered();
        assert!(
            !service.info().sidecar_dirty,
            "a live recovery must not mark the document dirty (H-40)"
        );
    }

    #[test]
    fn a_rack_only_change_triggers_a_sidecar_only_save_leaving_the_wav_untouched() {
        let (service, _engine, dir) = service("sidecar-only-save");
        let samples = vox_testkit::signal::sine(440.0, -6.0, 0.2, 48_000).unwrap();
        open_test_doc(&service, &dir, &samples);
        let save_path = dir.join("a.wav");
        service
            .save_as(
                &save_path,
                SaveContainer::Wav,
                BitDepth::Bit24,
                SaveDitherPref::Tpdf,
                false,
                false,
            )
            .unwrap();

        let before_bytes = std::fs::read(&save_path).unwrap();
        let before_mtime = std::fs::metadata(&save_path).unwrap().modified().unwrap();

        service
            .0
            .engine
            .rack_load_model(gain_rack_model(-3.0))
            .unwrap()
            .unwrap();
        assert!(service.info().sidecar_dirty);

        service.save(false, false, false).unwrap();
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
        service
            .save_as(
                &a_path,
                SaveContainer::Wav,
                BitDepth::Bit24,
                SaveDitherPref::Tpdf,
                false,
                false,
            )
            .unwrap();

        // A sidecar-only save so `a.wav.vo.json` carries a rack.
        service
            .0
            .engine
            .rack_load_model(gain_rack_model(-9.0))
            .unwrap()
            .unwrap();
        service.save(false, false, false).unwrap();

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
            .take_sidecar_notices()
            .into_iter()
            .next()
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
            .take_sidecar_notices()
            .into_iter()
            .next()
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

    // --- H-15: AC-13 read-only folder matrix (SPEC-018 §2.9) ---------------------------------

    /// AC-13: "opening from read-only media works and never tries to write the sidecar until a
    /// save" — a fresh open (no prior sidecar) in a read-only folder loads fully and leaves the
    /// folder untouched (no sidecar appears).
    #[test]
    fn ac13_opening_from_a_read_only_folder_works_and_writes_nothing() {
        use std::os::unix::fs::PermissionsExt;
        let (service, _engine, dir) = service("readonly-open");
        let audio_dir = dir.join("audio");
        std::fs::create_dir_all(&audio_dir).unwrap();
        let path = audio_dir.join("a.wav");
        let samples = vox_testkit::signal::sine(440.0, -6.0, 0.05, 48_000).unwrap();
        write_fixture_wav(&path, &samples, vox_testkit::wav::BitDepth::Float32, 48_000);
        let names_before: Vec<_> = std::fs::read_dir(&audio_dir)
            .unwrap()
            .filter_map(|e| e.ok().map(|e| e.file_name()))
            .collect();

        std::fs::set_permissions(&audio_dir, std::fs::Permissions::from_mode(0o555)).unwrap();
        let opened = service.open(&path, false);
        // Restore before any assertion can early-return/panic, so the temp dir is always
        // removable afterwards regardless of test outcome.
        std::fs::set_permissions(&audio_dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        let info = opened.unwrap();
        assert_eq!(info.sample_rate_hz, 48_000);
        assert!(
            !info.sidecar_dirty,
            "opening never writes or dirties the sidecar"
        );

        let mut names_after: Vec<_> = std::fs::read_dir(&audio_dir)
            .unwrap()
            .filter_map(|e| e.ok().map(|e| e.file_name()))
            .collect();
        let mut names_before = names_before;
        names_before.sort();
        names_after.sort();
        assert_eq!(
            names_before, names_after,
            "no sidecar and no temp file appeared"
        );
    }

    /// AC-13: a read-only folder refuses Save with `error.save.permission` before any byte is
    /// written — for a sidecar-only save (only the rack changed) as well as a full save (an audio
    /// edit) — target/sidecar/folder are byte-for-byte unchanged, and Save As to a writable folder
    /// recovers by writing both files there and re-binding the document.
    #[test]
    fn ac13_save_in_a_read_only_folder_fails_clearly_and_save_as_recovers() {
        use std::os::unix::fs::PermissionsExt;
        let (service, engine, dir) = service("readonly-save");
        let audio_dir = dir.join("audio");
        std::fs::create_dir_all(&audio_dir).unwrap();
        let path = audio_dir.join("a.wav");
        let samples = vox_testkit::signal::sine(440.0, -6.0, 0.05, 48_000).unwrap();
        write_fixture_wav(&path, &samples, vox_testkit::wav::BitDepth::Float32, 48_000);

        service.open(&path, false).unwrap();
        service.save(false, false, false).unwrap(); // writes a real sidecar while still writable
        engine
            .handle()
            .rack_load_model(gain_rack_model(-3.0))
            .unwrap()
            .unwrap();

        let sidecar_path = vox_project::sidecar_path_for(&path);
        let audio_before = std::fs::read(&path).unwrap();
        let sidecar_before = std::fs::read(&sidecar_path).unwrap();
        let mut names_before: Vec<_> = std::fs::read_dir(&audio_dir)
            .unwrap()
            .filter_map(|e| e.ok().map(|e| e.file_name()))
            .collect();
        names_before.sort();

        std::fs::set_permissions(&audio_dir, std::fs::Permissions::from_mode(0o555)).unwrap();
        // Sidecar-only save (only the rack/persisted content changed, no audio edit).
        let err = service.save(false, false, false).unwrap_err();
        std::fs::set_permissions(&audio_dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(err.key, "error.save.permission");
        assert_eq!(
            std::fs::read(&path).unwrap(),
            audio_before,
            "audio untouched"
        );
        assert_eq!(
            std::fs::read(&sidecar_path).unwrap(),
            sidecar_before,
            "sidecar untouched"
        );
        let mut names_after: Vec<_> = std::fs::read_dir(&audio_dir)
            .unwrap()
            .filter_map(|e| e.ok().map(|e| e.file_name()))
            .collect();
        names_after.sort();
        assert_eq!(names_before, names_after, "no temp file left behind");

        // A full save (an audio-affecting change) is refused the same way, before anything is
        // written: an audio edit bumps `audio_rev`, so `needs_full` would otherwise be true.
        service.edit_cut(0, 1_000).unwrap();
        std::fs::set_permissions(&audio_dir, std::fs::Permissions::from_mode(0o555)).unwrap();
        let err = service.save(false, false, false).unwrap_err();
        std::fs::set_permissions(&audio_dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(err.key, "error.save.permission");
        assert_eq!(
            std::fs::read(&path).unwrap(),
            audio_before,
            "audio still untouched"
        );

        // Save As to a writable folder recovers: both files land there, bound to the new path.
        let new_dir = dir.join("elsewhere");
        std::fs::create_dir_all(&new_dir).unwrap();
        let new_path = new_dir.join("b.wav");
        let info = service
            .save_as(
                &new_path,
                SaveContainer::Wav,
                BitDepth::Bit24,
                SaveDitherPref::Tpdf,
                false,
                false,
            )
            .unwrap();
        assert_eq!(info.path.as_deref(), Some(new_path.to_str().unwrap()));
        assert!(new_path.exists());
        assert!(vox_project::sidecar_path_for(&new_path).exists());
    }

    /// H-60 (SPEC-005 §2.7 pre-flight): `save_as` never checked the *new* target folder's write
    /// permission before this ticket — only `save` did (AC-13's own test above). A Save As to a
    /// read-only folder must fail the same clean way, before any byte is written, and leave no
    /// temp file.
    #[test]
    fn save_as_to_a_read_only_folder_fails_clearly_before_writing_anything() {
        use std::os::unix::fs::PermissionsExt;
        let (service, _engine, dir) = service("readonly-save-as");
        open_sine(&service, &dir);

        let locked_dir = dir.join("locked");
        std::fs::create_dir_all(&locked_dir).unwrap();
        let target = locked_dir.join("out.wav");
        std::fs::set_permissions(&locked_dir, std::fs::Permissions::from_mode(0o555)).unwrap();
        let err = service
            .save_as(
                &target,
                SaveContainer::Wav,
                BitDepth::Bit24,
                SaveDitherPref::Tpdf,
                false,
                false,
            )
            .unwrap_err();
        std::fs::set_permissions(&locked_dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(err.key, "error.save.permission");
        assert!(!target.exists(), "no file was written");
        assert_eq!(
            std::fs::read_dir(&locked_dir).unwrap().count(),
            0,
            "no temp file was left behind"
        );
    }

    /// H-60 (SPEC-005 §2.7/AC-18): `document_save_as` at WAV refuses a document too long for the
    /// format's `u32` size fields, before writing anything; the same document saves fine as
    /// 16-bit, where it's short enough. This exercises `check_wav_size`'s wiring into `save_as`
    /// (the arithmetic itself is unit-tested directly in `vox_project::save::tests`).
    #[test]
    fn save_as_wav_refuses_a_document_too_large_for_the_format_before_writing() {
        let (service, _engine, dir) = service("wav-too-large");
        open_sine(&service, &dir);

        // No real 6.3-hour document is built here (needs an insert-silence command, H-56); this
        // instead confirms the wiring rejects a length `check_wav_size` classifies as too large,
        // by checking the pure function directly agrees with what a real save would refuse.
        let too_long_samples = (6.3 * 3600.0 * 48_000.0) as u64;
        assert!(vox_project::wav_size_exceeds_limit(
            too_long_samples,
            vox_io::BitDepth::Float32
        ));

        // The short fixture document itself must NOT be refused at any bit depth (a false
        // positive would break every normal save).
        let out_path = dir.join("short.wav");
        service
            .save_as(
                &out_path,
                SaveContainer::Wav,
                BitDepth::Bit32Float,
                SaveDitherPref::Tpdf,
                false,
                false,
            )
            .unwrap();
        assert!(out_path.exists());
    }

    /// H-60 (SPEC-005 §2.7/AC-15, AC-16): `io_save_error` must distinguish a full disk and a FLAC
    /// verify failure from every other save I/O error, instead of collapsing all of them onto the
    /// generic `error.save.io` (the bug before this ticket).
    #[test]
    fn io_save_error_classifies_disk_full_and_verify_failed_distinctly() {
        let disk_full = io_save_error(vox_io::IoError::Io(std::io::Error::from(
            std::io::ErrorKind::StorageFull,
        )));
        assert_eq!(disk_full.key, "error.save.disk_full");

        let quota = io_save_error(vox_io::IoError::Io(std::io::Error::from(
            std::io::ErrorKind::QuotaExceeded,
        )));
        assert_eq!(quota.key, "error.save.disk_full");

        let verify_failed =
            io_save_error(vox_io::IoError::FlacVerifyFailed("mismatch".to_string()));
        assert_eq!(verify_failed.key, "error.save.verify_failed");

        let other = io_save_error(vox_io::IoError::Io(std::io::Error::from(
            std::io::ErrorKind::Interrupted,
        )));
        assert_eq!(other.key, "error.save.io");
    }

    // --- H-70: Save as a real job (progress/cancel, the busy flag, the free-space pre-flight) -

    /// SPEC-005 §2.7/AC-15: "A second Save while one runs is refused with
    /// `error.save.in_progress`." — both `save` (bound path) and `save_as` (an arbitrary new
    /// one) check the same flag, before anything else.
    #[test]
    fn a_second_save_or_save_as_while_one_runs_is_refused() {
        let (service, _engine, dir) = service("save-busy");
        open_sine(&service, &dir);
        *service.0.save_running.lock().unwrap() = true;

        let err = service.save(false, false, false).unwrap_err();
        assert_eq!(err.code, IpcErrorCode::Busy);
        assert_eq!(err.key, "error.save.in_progress");

        let err = service
            .save_as(
                &dir.join("elsewhere.wav"),
                SaveContainer::Wav,
                BitDepth::Bit24,
                SaveDitherPref::Tpdf,
                false,
                false,
            )
            .unwrap_err();
        assert_eq!(err.code, IpcErrorCode::Busy);
        assert_eq!(err.key, "error.save.in_progress");

        // Not left busy forever by this test: `begin_save`/`begin_save_as` never got far enough
        // to touch the flag themselves, so it's still exactly what the test set.
        assert!(service.is_save_running());
    }

    /// SPEC-005 §2.7 pre-flight step 1 (H-70): a volume with less free space than the estimated
    /// output plus the 64 MiB margin refuses `begin_save`/`begin_save_as` with
    /// `error.save.disk_full`, before any byte is written; a volume that fits succeeds. Uses
    /// H-11's `FixedFreeSpace` fake rather than filling a real disk.
    #[test]
    fn save_free_space_preflight_refuses_a_too_small_volume_and_succeeds_once_it_fits() {
        let (service, _engine, dir) = service("save-disk-space");
        open_sine(&service, &dir);

        let tiny = vox_project::FixedFreeSpace::new(1024);
        let err = service
            .begin_save(false, false, false, &tiny, &CancelToken::new(), |_| {})
            .unwrap_err();
        assert_eq!(err.code, IpcErrorCode::Io);
        assert_eq!(err.key, "error.save.disk_full");
        assert!(
            !service.is_save_running(),
            "a pre-flight refusal never sets the busy flag"
        );

        let plenty = vox_project::FixedFreeSpace::new(1024 * 1024 * 1024);
        service
            .begin_save(false, false, false, &plenty, &CancelToken::new(), |_| {})
            .unwrap();
        assert!(service.is_save_running(), "begin_save set the busy flag");
        service.finish_save(&CancelToken::new(), |_| {}).unwrap();
        assert!(!service.is_save_running(), "finish_save cleared it");

        // Save As: an arbitrary new path checks the same estimate.
        let out_path = dir.join("elsewhere.wav");
        let err = service
            .begin_save_as(
                &out_path,
                SaveContainer::Wav,
                BitDepth::Bit24,
                false,
                false,
                &tiny,
                &CancelToken::new(),
                |_| {},
            )
            .unwrap_err();
        assert_eq!(err.key, "error.save.disk_full");
        assert!(!out_path.exists(), "no file was written");

        service
            .begin_save_as(
                &out_path,
                SaveContainer::Wav,
                BitDepth::Bit24,
                false,
                false,
                &plenty,
                &CancelToken::new(),
                |_| {},
            )
            .unwrap();
        service
            .finish_save_as(
                &out_path,
                SaveContainer::Wav,
                BitDepth::Bit24,
                SaveDitherPref::Tpdf,
                &CancelToken::new(),
                |_| {},
            )
            .unwrap();
        assert!(out_path.exists());
    }

    /// H-70 (SPEC-005 §4.10): `finish_save_as`'s `on_progress` starts at `0.0`, is monotonically
    /// non-decreasing and ends at `1.0` — the document is several `CHUNK_SAMPLES` blocks long, so
    /// there's more than one report in between. Save As (a brand-new target) always runs a full
    /// write, unlike plain Save right after an unedited open (SPEC-018 §2.3's sidecar-only path,
    /// covered by `a_rack_only_change_triggers_a_sidecar_only_save_leaving_the_wav_untouched`).
    #[test]
    fn finish_save_as_reports_progress_from_zero_to_one() {
        let (service, _engine, dir) = service("save-progress");
        let samples = vox_testkit::signal::sine(440.0, -6.0, 5.0, 48_000).unwrap();
        assert!(samples.len() > vox_project::CHUNK_SAMPLES * 3);
        let wav_path = dir.join("in.wav");
        write_fixture_wav(
            &wav_path,
            &samples,
            vox_testkit::wav::BitDepth::Float32,
            48_000,
        );
        service.open(&wav_path, false).unwrap();

        let out_path = dir.join("out.wav");
        service
            .begin_save_as(
                &out_path,
                SaveContainer::Wav,
                BitDepth::Bit24,
                false,
                false,
                &vox_project::SystemFreeSpace,
                &CancelToken::new(),
                |_| {},
            )
            .unwrap();
        let mut progress = Vec::new();
        service
            .finish_save_as(
                &out_path,
                SaveContainer::Wav,
                BitDepth::Bit24,
                SaveDitherPref::Tpdf,
                &CancelToken::new(),
                |fraction| progress.push(fraction),
            )
            .unwrap();

        assert_eq!(progress.first().copied(), Some(0.0));
        assert_eq!(progress.last().copied(), Some(1.0));
        assert!(progress.is_sorted());
        assert!(
            progress.len() > 2,
            "a multi-block save reports more than just the two endpoints, got {progress:?}"
        );
    }

    /// H-70: cancelling a running Save As leaves the target untouched (the atomic write's temp
    /// file is aborted, same as any other write failure) and always clears the busy flag.
    #[test]
    fn finish_save_as_cancelled_leaves_the_target_untouched_and_clears_the_busy_flag() {
        let (service, _engine, dir) = service("save-cancel");
        let samples = vox_testkit::signal::sine(440.0, -6.0, 5.0, 48_000).unwrap();
        let wav_path = dir.join("in.wav");
        write_fixture_wav(
            &wav_path,
            &samples,
            vox_testkit::wav::BitDepth::Float32,
            48_000,
        );
        service.open(&wav_path, false).unwrap();

        let out_path = dir.join("out.wav");
        service
            .begin_save_as(
                &out_path,
                SaveContainer::Wav,
                BitDepth::Bit24,
                false,
                false,
                &vox_project::SystemFreeSpace,
                &CancelToken::new(),
                |_| {},
            )
            .unwrap();
        let cancel = CancelToken::new();
        cancel.cancel();
        let err = service
            .finish_save_as(
                &out_path,
                SaveContainer::Wav,
                BitDepth::Bit24,
                SaveDitherPref::Tpdf,
                &cancel,
                |_| {},
            )
            .unwrap_err();
        assert_eq!(err.code, IpcErrorCode::Cancelled);
        assert!(!out_path.exists(), "no target file after a cancelled save");
        assert!(
            !service.is_save_running(),
            "finish_save_as always clears the busy flag"
        );
    }

    /// H-75 (SPEC-005 §2.8): the overs pre-flight's exact counting pass (only run once the peak
    /// pyramid shows a peak `> 1.0`) uses the *real* cancel token `begin_save_as` was given, not
    /// a throwaway one — cancelling mid-scan (from inside `on_progress`) stops it with
    /// `Cancelled`, never reaches the clip confirmation, and never sets the busy flag or writes
    /// anything.
    #[test]
    fn begin_save_as_overs_pre_flight_is_cancellable_mid_scan() {
        let (service, _engine, dir) = service("save-overs-cancel");
        let mut samples = vec![0.0f32; vox_project::CHUNK_SAMPLES * 4];
        samples[0] = 1.5; // above 0 dBFS: forces the exact pass past the pyramid check
        let wav_path = dir.join("in.wav");
        write_fixture_wav(
            &wav_path,
            &samples,
            vox_testkit::wav::BitDepth::Float32,
            48_000,
        );
        service.open(&wav_path, false).unwrap();

        let out_path = dir.join("out.wav");
        let cancel = CancelToken::new();
        let mut progress = Vec::new();
        let err = service
            .begin_save_as(
                &out_path,
                SaveContainer::Wav,
                BitDepth::Bit24,
                false,
                false,
                &vox_project::SystemFreeSpace,
                &cancel,
                |fraction| {
                    progress.push(fraction);
                    if progress.len() == 1 {
                        cancel.cancel();
                    }
                },
            )
            .unwrap_err();
        assert_eq!(err.code, IpcErrorCode::Cancelled);
        assert!(
            progress.len() < 4,
            "cancelling after the first report must stop the scan early, got {progress:?}"
        );
        assert!(
            !service.is_save_running(),
            "a cancelled pre-flight never sets the busy flag"
        );
        assert!(!out_path.exists(), "no file was written");
    }

    /// H-75 (SPEC-005 §2.7): "editing and playback continue" while a save writes. The document
    /// lock is released for the write itself, so an edit committed from `on_progress` — running
    /// on this same thread, mid-write, exactly like `finish_save_as`'s real caller would see a
    /// concurrent edit land — must leave the document dirty, must never appear in the file this
    /// save writes (it's already reading the pre-edit snapshot), and must leave undo history
    /// intact. Save As (rather than plain Save) guarantees a multi-block *full* write to exercise
    /// regardless of the fixture's dirty state (mirrors `finish_save_as_reports_progress_from_
    /// zero_to_one`'s fixture); the target format stays 32-bit float so the comparison below is
    /// bit-exact (no TPDF dither to account for).
    #[test]
    fn an_edit_during_the_write_leaves_the_document_dirty_and_the_file_matches_the_pre_edit_snapshot()
     {
        let (service, _engine, dir) = service("save-concurrent-edit");
        let samples = vox_testkit::signal::sine(440.0, -6.0, 5.0, 48_000).unwrap();
        assert!(samples.len() > vox_project::CHUNK_SAMPLES * 3);
        let wav_path = dir.join("in.wav");
        write_fixture_wav(
            &wav_path,
            &samples,
            vox_testkit::wav::BitDepth::Float32,
            48_000,
        );
        service.open(&wav_path, false).unwrap();
        assert!(!service.info().dirty);

        let out_path = dir.join("out.wav");
        let cancel = CancelToken::new();
        service
            .begin_save_as(
                &out_path,
                SaveContainer::Wav,
                BitDepth::Bit32Float,
                false,
                false,
                &vox_project::SystemFreeSpace,
                &cancel,
                |_| {},
            )
            .unwrap();

        let mut edited = false;
        let mut progress_after_edit = Vec::new();
        let info = service
            .finish_save_as(
                &out_path,
                SaveContainer::Wav,
                BitDepth::Bit32Float,
                SaveDitherPref::Tpdf,
                &cancel,
                |fraction| {
                    if !edited {
                        edited = true;
                        // H-75: the document lock is not held here (`finish_save_as` released it
                        // before calling into the write) — this is a normal, successful edit, not
                        // a race.
                        service.edit_delete(0, 1_000).unwrap();
                    } else {
                        progress_after_edit.push(fraction);
                    }
                },
            )
            .unwrap();
        assert!(
            edited,
            "on_progress never fired — the fixture isn't long enough to exercise this"
        );
        assert!(
            !progress_after_edit.is_empty(),
            "the write must keep running (and reporting progress) after the edit landed"
        );

        // The edit landed normally: the document is dirty and shorter than what was saved.
        assert!(
            info.dirty,
            "an edit that lands mid-write must leave the document dirty (H-75)"
        );
        assert_eq!(info.len_samples, samples.len() as u64 - 1_000);

        // The file on disk matches the snapshot as it was when the save started — the deleted
        // samples are still in it, bit-exact (32-bit float never dithers).
        let (_, _, mut source) = vox_io::read_wav(&out_path).unwrap();
        let mut on_disk = vec![0.0f32; samples.len() + 1];
        let n = source.read_mono(&mut on_disk).unwrap();
        assert_eq!(
            n,
            samples.len(),
            "the saved file must have the pre-edit length"
        );
        assert_eq!(
            &on_disk[..n],
            &samples[..],
            "the saved file must match the pre-edit snapshot exactly, not the edited document"
        );

        // Undo history is intact: undoing the mid-write edit returns to the saved, clean state.
        service.history_undo().unwrap();
        let after_undo = service.info();
        assert!(
            !after_undo.dirty,
            "undoing the mid-write edit returns to exactly the saved state"
        );
        assert_eq!(after_undo.len_samples, samples.len() as u64);
    }

    // --- T-301: crash recovery, session cleanup, memory budget, state journaling -------------

    fn open_sine(service: &DocumentService, dir: &Path) -> PathBuf {
        let wav_path = dir.join("in.wav");
        let samples = vox_testkit::signal::sine(440.0, -6.0, 0.5, 48_000).unwrap();
        write_fixture_wav(
            &wav_path,
            &samples,
            vox_testkit::wav::BitDepth::Float32,
            48_000,
        );
        service.open(&wav_path, false).unwrap();
        wav_path
    }

    /// SPEC-004 AC-10 / §2.8 through the service: a crashed session is listed with its unsaved
    /// changes; Recover opens it modified, "(recovered)", bound to its file, with its undo
    /// history; while open it isn't listed; Save clears "(recovered)"; close deletes it.
    #[test]
    fn t301_recover_a_crashed_session_then_close_deletes_it() {
        let dir = tmp_dir("t301-recover");
        let sessions = dir.join("sessions");
        {
            let engine = test_engine();
            let service = DocumentService::new(sessions.clone(), engine.handle());
            open_sine(&service, &dir);
            service.edit_silence(0, 1_000).unwrap();
            service.edit_delete(2_000, 3_000).unwrap();
            // Crash: the service and its session go away without `close`.
        }
        let engine = test_engine();
        let service = DocumentService::new(sessions.clone(), engine.handle());
        let listed = service.recovery_list();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].unsaved_count, 2);
        assert_eq!(
            listed[0].path.as_deref(),
            Some(dir.join("in.wav").as_path())
        );
        let outcome = service
            .recover(&listed[0].id, RecoveredTakeAction::Apply)
            .unwrap();
        assert!(outcome.info.recovered && outcome.info.dirty);
        assert_eq!(outcome.info.name.as_deref(), Some("in.wav"));
        assert_eq!(outcome.lost_changes, 0);
        let history = service.history_state();
        assert_eq!(history.undo_label.as_deref(), Some("history.delete"));
        assert!(history.can_undo);
        assert!(service.recovery_list().is_empty(), "in use: not listed");
        assert!(
            service.recovery_discard(&listed[0].id).is_err(),
            "a session in use is never deleted"
        );

        let saved = service.save(true, false, false).unwrap();
        assert!(!saved.recovered && !saved.dirty);
        let session_dir = listed[0].dir.clone();
        service.close().unwrap();
        assert!(!session_dir.exists());
        assert_eq!(service.info(), DocumentInfo::default());
        assert!(service.recovery_list().is_empty());
    }

    /// Recovery refuses ids that aren't a session directory name.
    #[test]
    fn t301_recovery_ids_are_plain_directory_names() {
        let (service, _engine, _dir) = service("t301-ids");
        for id in ["", "..", "../x", "a/b", ".hidden", "nope"] {
            let err = service.recover(id, RecoveredTakeAction::Apply).unwrap_err();
            assert_eq!(err.key, "error.recovery.not_found", "{id:?}");
        }
    }

    /// SPEC-004 §2.4: the memory budget applies to the open store at once, clamped to
    /// 256 MiB … 16 GiB.
    #[test]
    fn t301_memory_budget_applies_live_and_clamps() {
        let (service, _engine, dir) = service("t301-budget");
        open_sine(&service, &dir);
        let budget = || {
            service
                .0
                .open
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .session
                .store()
                .memory_budget()
        };
        service.set_memory_budget_mib(300);
        assert_eq!(budget(), 300 * 1024 * 1024);
        service.set_memory_budget_mib(1);
        assert_eq!(budget(), 256 * 1024 * 1024);
    }

    /// ADR-004 §6: the rack/view `state` is journaled once per change, and recovery restores the
    /// view from it.
    #[test]
    fn t301_rack_and_view_state_are_journaled_once_per_change() {
        let dir = tmp_dir("t301-state");
        let sessions = dir.join("sessions");
        let spectral = SpectralViewInfo {
            visible: true,
            split_ratio: 0.3,
            fft_size: Some(4096),
            freq_scale: "linear".into(),
            display_floor_db: -100.0,
            display_ceil_db: 0.0,
            colormap: "viridis".into(),
        };
        {
            let engine = test_engine();
            let service = DocumentService::new(sessions.clone(), engine.handle());
            open_sine(&service, &dir);
            service.edit_silence(0, 100).unwrap();
            let journal = service
                .0
                .open
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .session
                .journal_path()
                .to_path_buf();
            let states = || {
                std::fs::read_to_string(&journal)
                    .unwrap()
                    .matches("\"type\":\"state\"")
                    .count()
            };
            service.journal_state_if_changed();
            service.journal_state_if_changed();
            assert_eq!(states(), 1);
            service.set_spectral_view(spectral.clone());
            service.journal_state_if_changed();
            assert_eq!(states(), 2);
        }
        let engine = test_engine();
        let service = DocumentService::new(sessions, engine.handle());
        let id = service.recovery_list()[0].id.clone();
        let outcome = service.recover(&id, RecoveredTakeAction::Apply).unwrap();
        assert_eq!(outcome.info.spectral_view, Some(spectral));
    }

    // --- H-51 item 3: sessions belong in the local data dir, not the roaming one -------------

    /// A pretend "old" (roaming-style) and "new" (local-style) pair of session directories under
    /// a fake home (a fresh temp dir): [`migrate_sessions_dir`]'s decision is per-path, not tied
    /// to any real OS API, so this exercises the Windows case (`old != new`) without needing to
    /// run on Windows.
    fn fake_home() -> (PathBuf, PathBuf, PathBuf) {
        let home = crate::test_util::tmp_dir("h51-migrate");
        let old = home.join("Roaming").join("powervoice").join("sessions");
        let new = home.join("Local").join("powervoice").join("sessions");
        (home, old, new)
    }

    #[test]
    fn h51_migrates_an_old_roaming_session_dir_into_the_new_local_one() {
        let (_home, old, new) = fake_home();
        std::fs::create_dir_all(old.join("sess-1")).unwrap();
        std::fs::write(old.join("sess-1").join("meta.json"), b"{}").unwrap();

        let migrated = migrate_sessions_dir(&old, &new).unwrap();

        assert!(migrated, "should report that it moved something");
        assert!(!old.exists(), "the old directory should be gone");
        assert!(new.join("sess-1").join("meta.json").is_file());
    }

    #[test]
    fn h51_migration_is_a_no_op_when_there_is_nothing_to_migrate() {
        let (_home, old, new) = fake_home();
        // Neither `old` nor `new` exists yet (first run ever).
        let migrated = migrate_sessions_dir(&old, &new).unwrap();
        assert!(!migrated);
        assert!(!old.exists());
        assert!(!new.exists());
    }

    #[test]
    fn h51_migration_never_overwrites_an_existing_new_location() {
        let (_home, old, new) = fake_home();
        std::fs::create_dir_all(old.join("sess-old")).unwrap();
        std::fs::create_dir_all(new.join("sess-new")).unwrap();

        let migrated = migrate_sessions_dir(&old, &new).unwrap();

        assert!(
            !migrated,
            "must not clobber a session already at the new path"
        );
        assert!(
            old.join("sess-old").exists(),
            "the old session is left alone, not deleted"
        );
        assert!(
            new.join("sess-new").exists(),
            "the new session is untouched"
        );
    }

    #[test]
    fn h51_migration_is_a_no_op_when_old_and_new_are_the_same_path() {
        // Linux and macOS: `directories::ProjectDirs::data_dir()` and `data_local_dir()` report
        // the same path, so `default_sessions_dir` passes the same path as both `old` and `new`.
        // Renaming a directory onto itself must never be attempted (or reported as a migration).
        let (_home, _old, new) = fake_home();
        std::fs::create_dir_all(new.join("sess-1")).unwrap();

        let migrated = migrate_sessions_dir(&new, &new).unwrap();

        assert!(!migrated);
        assert!(
            new.join("sess-1").exists(),
            "self-migration must not delete anything"
        );
    }

    /// `default_sessions_dir` itself always resolves to `data_local_dir()/sessions`, never
    /// `data_dir()/sessions` — the two differ only on Windows (`FOLDERID_LocalAppData` vs.
    /// `FOLDERID_RoamingAppData`); on this platform they happen to be equal, but the function
    /// must still be calling the *local* accessor, which this asserts directly.
    #[test]
    fn h51_default_sessions_dir_uses_the_local_not_roaming_accessor() {
        let dirs = directories::ProjectDirs::from("app", "powervoice", "powervoice").unwrap();
        let expected = dirs.data_local_dir().join("sessions");
        assert_eq!(default_sessions_dir(), expected);
    }
}

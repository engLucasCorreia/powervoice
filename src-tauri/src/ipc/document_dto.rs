//! Document DTOs (S1-03): the open-document snapshot the UI shows (title, File menu, waveform
//! header) and the `peaks_get` request shape (the response itself is raw `VXPK` bytes over
//! `ipc::Response`, ADR-003 §2 — never JSON floats, CLAUDE.md).

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::document::{
    ClipboardInfo, DocumentInfo, EditResult, HistoryState, MarkerInfo, MarkerRangeEditKind,
    PasteTarget, SpectralViewInfo,
};

/// `document_changed` event payload, and the result of `document_open`/`document_save`/
/// `document_save_as`. `name: None` means no document is open.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "bindings.ts")]
pub struct DocumentDto {
    pub name: Option<String>,
    pub path: Option<String>,
    pub sample_rate_hz: u32,
    pub len_samples: u64,
    pub dirty: bool,
    pub audio_rev: u64,
    /// T-306 (SPEC-018 §2.4): `*` and the close/quit prompts fire on `dirty || sidecar_dirty`.
    pub sidecar_dirty: bool,
    /// T-306 (SPEC-018 §2.6.5): the sidecar's spectral-pane settings, if any (`null` = keep the
    /// UI's current/last-used settings, SPEC-007 §2.1).
    pub spectral_view: Option<SpectralViewDto>,
}

impl From<DocumentInfo> for DocumentDto {
    fn from(info: DocumentInfo) -> Self {
        Self {
            name: info.name,
            path: info.path,
            sample_rate_hz: info.sample_rate_hz,
            len_samples: info.len_samples,
            dirty: info.dirty,
            audio_rev: info.audio_rev,
            sidecar_dirty: info.sidecar_dirty,
            spectral_view: info.spectral_view.map(SpectralViewDto::from),
        }
    }
}

/// T-306 (SPEC-018 §2.6.5's `view.spectral`, SPEC-007 §3): per-document spectral pane settings.
/// `fft_size: null` means Auto (SPEC-007 §2.6).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "bindings.ts")]
pub struct SpectralViewDto {
    pub visible: bool,
    pub split_ratio: f64,
    pub fft_size: Option<u32>,
    pub freq_scale: String,
    pub display_floor_db: f64,
    pub display_ceil_db: f64,
    pub colormap: String,
}

impl From<SpectralViewInfo> for SpectralViewDto {
    fn from(v: SpectralViewInfo) -> Self {
        Self {
            visible: v.visible,
            split_ratio: v.split_ratio,
            fft_size: v.fft_size,
            freq_scale: v.freq_scale,
            display_floor_db: v.display_floor_db,
            display_ceil_db: v.display_ceil_db,
            colormap: v.colormap,
        }
    }
}

impl From<SpectralViewDto> for SpectralViewInfo {
    fn from(v: SpectralViewDto) -> Self {
        Self {
            visible: v.visible,
            split_ratio: v.split_ratio,
            fft_size: v.fft_size,
            freq_scale: v.freq_scale,
            display_floor_db: v.display_floor_db,
            display_ceil_db: v.display_ceil_db,
            colormap: v.colormap,
        }
    }
}

/// S2-01: the result of a cut/copy/paste/delete/trim/silence command or an undo/redo (SPEC-008
/// §4.3's `EditResult`, minus `rev`/`base_rev` — revision-guarded commands are deferred).
/// `changed: false` only for Copy, a whole-document Trim, and undo/redo at the history's edge.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "bindings.ts")]
pub struct EditResultDto {
    pub changed: bool,
    pub audio_rev: u64,
    pub len_samples: u64,
    /// `[start, end)` document samples, or `null` (no selection).
    pub selection: Option<(u64, u64)>,
    pub playhead_samples: u64,
}

impl From<EditResult> for EditResultDto {
    fn from(r: EditResult) -> Self {
        Self {
            changed: r.changed,
            audio_rev: r.audio_rev,
            len_samples: r.len_samples,
            selection: r.selection,
            playhead_samples: r.playhead_samples,
        }
    }
}

/// S2-01: where `edit_paste` inserts or replaces (SPEC-008 §2.1). The UI resolves "no selection"
/// to `Cursor` itself (SPEC-008 §2.2: an empty selection counts as the cursor).
#[derive(Debug, Clone, Copy, Deserialize, Serialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(export, export_to = "bindings.ts", rename_all = "snake_case")]
pub enum EditTargetDto {
    Cursor {
        at_samples: u64,
    },
    Range {
        start_samples: u64,
        end_samples: u64,
    },
}

impl From<EditTargetDto> for PasteTarget {
    fn from(dto: EditTargetDto) -> Self {
        match dto {
            EditTargetDto::Cursor { at_samples } => PasteTarget::Cursor(at_samples),
            EditTargetDto::Range {
                start_samples,
                end_samples,
            } => PasteTarget::Range(start_samples, end_samples),
        }
    }
}

/// S2-01: the Edit menu's Undo/Redo state (`history_state` event). Labels are i18n keys
/// (`history.cut`, …, CLAUDE.md), `null` when there's nothing to undo/redo.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "bindings.ts")]
pub struct HistoryStateDto {
    pub can_undo: bool,
    pub can_redo: bool,
    pub undo_label: Option<String>,
    pub redo_label: Option<String>,
}

impl From<HistoryState> for HistoryStateDto {
    fn from(s: HistoryState) -> Self {
        Self {
            can_undo: s.can_undo,
            can_redo: s.can_redo,
            undo_label: s.undo_label,
            redo_label: s.redo_label,
        }
    }
}

/// S2-01: the clipboard's length/rate (`clipboard_changed` event); `null` fields mean it's empty
/// (SPEC-008 §4.3).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "bindings.ts")]
pub struct ClipboardChangedDto {
    pub len_samples: Option<u64>,
    pub sample_rate_hz: Option<u32>,
}

impl From<ClipboardInfo> for ClipboardChangedDto {
    fn from(c: ClipboardInfo) -> Self {
        Self {
            len_samples: c.len_samples,
            sample_rate_hz: c.sample_rate_hz,
        }
    }
}

/// S2-03: one marker (SPEC-009 §2.1's essential subset — no `kind`), as `markers_get` reports the
/// list and `marker_add` reports the created marker.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "bindings.ts")]
pub struct MarkerDto {
    pub id: u64,
    pub pos_samples: u64,
    pub len_samples: u64,
    pub name: String,
}

impl From<MarkerInfo> for MarkerDto {
    fn from(m: MarkerInfo) -> Self {
        Self {
            id: m.id,
            pos_samples: m.pos_samples,
            len_samples: m.len_samples,
            name: m.name,
        }
    }
}

/// S2-03: `marker_set_range`'s undo label (SPEC-009 §2.5) — Start keeps `len` (`Move`), End/
/// Duration keep `pos` (`Resize`).
#[derive(Debug, Clone, Copy, Deserialize, Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "bindings.ts", rename_all = "snake_case")]
pub enum MarkerRangeKindDto {
    Move,
    Resize,
}

impl From<MarkerRangeKindDto> for MarkerRangeEditKind {
    fn from(kind: MarkerRangeKindDto) -> Self {
        match kind {
            MarkerRangeKindDto::Move => MarkerRangeEditKind::Move,
            MarkerRangeKindDto::Resize => MarkerRangeEditKind::Resize,
        }
    }
}

/// `peaks_get`'s request (ADR-003 §2's `PeaksRequest`). `audio_rev` is the revision the UI last
/// saw; the server always answers with the document's *current* `audio_rev` regardless (in its
/// `VXPK` response header) — staleness is the caller's problem to detect (ADR-003: "the UI drops
/// any response whose `audio_rev` is not current"), not something this command refuses.
#[derive(Debug, Clone, Copy, Deserialize, Serialize, TS)]
#[ts(export, export_to = "bindings.ts")]
pub struct PeaksRequestDto {
    pub request_id: u32,
    pub audio_rev: u64,
    pub spp: u32,
    pub start_sample: u64,
    pub count: u32,
}

/// T-202: one source channel's role (SPEC-005 §2.4), part of `document_probe`'s payload — the
/// data a future channel-choice dialog (T-209) needs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "bindings.ts")]
pub struct ProbeChannelDto {
    pub label: String,
    pub is_lfe: bool,
}

/// T-202: `document_probe`'s result (SPEC-005 §2.3 step 1, §2.4). `channel_peaks_dbfs` is empty
/// for mono (no downmix choice applies); `-inf`/`NaN` peaks serialize as JSON `null`
/// (`serde_json`'s `float_roundtrip` behaviour, same convention as `loudness_dto`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "bindings.ts")]
pub struct DocumentProbeDto {
    pub container: String,
    pub codec: String,
    pub sample_rate_hz: u32,
    pub channels: Vec<ProbeChannelDto>,
    pub len_samples: Option<u64>,
    pub channel_peaks_dbfs: Vec<f64>,
    pub identical_channels: bool,
    pub suggested_channel: Option<u32>,
}

impl From<vox_project::ImportProbe> for DocumentProbeDto {
    fn from(p: vox_project::ImportProbe) -> Self {
        Self {
            container: p.container,
            codec: p.codec,
            sample_rate_hz: p.sample_rate_hz,
            channels: p
                .channels
                .into_iter()
                .map(|c| ProbeChannelDto {
                    label: c.label,
                    is_lfe: c.is_lfe,
                })
                .collect(),
            len_samples: p.len_samples,
            channel_peaks_dbfs: p.channel_peaks_dbfs.into_iter().map(f64::from).collect(),
            identical_channels: p.identical_channels,
            suggested_channel: p.suggested_channel.map(|i| i as u32),
        }
    }
}

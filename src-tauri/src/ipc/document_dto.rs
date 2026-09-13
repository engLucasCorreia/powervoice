//! Document DTOs (S1-03): the open-document snapshot the UI shows (title, File menu, waveform
//! header) and the `peaks_get` request shape (the response itself is raw `VXPK` bytes over
//! `ipc::Response`, ADR-003 §2 — never JSON floats, CLAUDE.md).

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::document::DocumentInfo;

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

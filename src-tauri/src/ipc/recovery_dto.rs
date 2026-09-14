//! Crash recovery & session storage DTOs (T-301, SPEC-004 §2.5, §2.7, §2.8).

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::document::{RecoverOutcome, RecoveredTakeAction};
use crate::ipc::document_dto::DocumentDto;

/// One recoverable session, as the start-up dialog and Settings → Recovery & storage list it
/// (SPEC-004 §2.7): `name`/`path` are `null` for an untitled recording ("Untitled recording").
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "bindings.ts")]
pub struct RecoverableSessionDto {
    pub id: String,
    pub name: Option<String>,
    pub path: Option<String>,
    /// The last edit time (the journal's mtime), ms since the Unix epoch.
    pub last_modified_unix_ms: Option<u64>,
    /// "3 unsaved changes".
    pub unsaved_changes: u64,
    /// A recording was in progress: its recovered length in samples.
    pub recording_samples: Option<u64>,
    pub sample_rate_hz: u32,
    /// "‹name› changed on disk since you opened it…".
    pub source_changed: bool,
    /// The bound file no longer exists (recovery still works; Save recreates it).
    pub source_missing: bool,
    /// The journal has a damaged tail: some of the latest changes may be lost.
    pub damaged: bool,
    pub size_bytes: u64,
}

impl From<&vox_project::gc::RecoverableSession> for RecoverableSessionDto {
    fn from(s: &vox_project::gc::RecoverableSession) -> Self {
        Self {
            id: s.id.clone(),
            name: s
                .path
                .as_ref()
                .and_then(|p| p.file_name())
                .map(|n| n.to_string_lossy().into_owned()),
            path: s.path.as_ref().map(|p| p.to_string_lossy().into_owned()),
            last_modified_unix_ms: s.last_modified.and_then(|t| {
                t.duration_since(std::time::UNIX_EPOCH)
                    .ok()
                    .map(|d| d.as_millis() as u64)
            }),
            unsaved_changes: s.unsaved_count,
            recording_samples: s.open_take.map(|_| s.open_take_samples),
            sample_rate_hz: s.sample_rate_hz.unwrap_or(0),
            source_changed: s.source_changed,
            source_missing: s.source_missing,
            damaged: s.journal_damaged,
            size_bytes: s.size_bytes,
        }
    }
}

/// What Recover does with an interrupted take (SPEC-004 §2.7; "apply" is the default).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "bindings.ts", rename_all = "snake_case")]
pub enum RecoveredTakeActionDto {
    Apply,
    NewDocument,
    Discard,
}

impl From<RecoveredTakeActionDto> for RecoveredTakeAction {
    fn from(a: RecoveredTakeActionDto) -> Self {
        match a {
            RecoveredTakeActionDto::Apply => RecoveredTakeAction::Apply,
            RecoveredTakeActionDto::NewDocument => RecoveredTakeAction::NewDocument,
            RecoveredTakeActionDto::Discard => RecoveredTakeAction::Discard,
        }
    }
}

/// `recovery_recover`'s result: the recovered document (modified, `recovered: true`) and how
/// many changes were lost (the "could not be recovered" notice also goes out as an event).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "bindings.ts")]
pub struct RecoverResultDto {
    pub document: DocumentDto,
    pub lost_changes: u32,
}

impl From<RecoverOutcome> for RecoverResultDto {
    fn from(o: RecoverOutcome) -> Self {
        Self {
            document: o.info.into(),
            lost_changes: u32::try_from(o.lost_changes).unwrap_or(u32::MAX),
        }
    }
}

/// Settings → Recovery & storage (SPEC-004 §2.5, §2.8): the open document's session storage
/// ("Session storage: 3.2 GB (history 2.1 GB)") and the recoverable sessions with their sizes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "bindings.ts")]
pub struct StorageInfoDto {
    /// `null` with no document open.
    pub session_bytes: Option<u64>,
    pub history_bytes: Option<u64>,
    pub sessions: Vec<RecoverableSessionDto>,
    /// Sum of the recoverable sessions' sizes ("Recovery data (X GB)").
    pub recovery_bytes: u64,
}

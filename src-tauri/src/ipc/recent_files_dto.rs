//! Recent files DTOs (T-306, SPEC-018 §2.12, §4.7).

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::settings::RecentFileEntry;

/// One `File → Open Recent` entry. `name`/`folder` are split out so the menu can show
/// "‹name› — ‹folder›" without re-deriving them in the UI; `exists` is resolved within the
/// existence-check budget (SPEC-018 §2.12) — `null` only if the check couldn't complete in time
/// (a stalled network mount), in which case the entry is shown as if it existed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "bindings.ts")]
pub struct RecentFileDto {
    pub path: String,
    pub name: String,
    pub folder: String,
    pub exists: Option<bool>,
}

impl RecentFileDto {
    /// Builds the DTO with `exists` already resolved by the caller — `recent_files_commands`'s
    /// bounded, off-thread existence check (SPEC-018 §2.12 `recent_check_budget_ms`) for
    /// `recent_files_get`, or an immediate check for `recent_files_remove`/`_clear`'s returned
    /// list (not on the menu-open path, so no budget concern there).
    pub fn from_entry(entry: &RecentFileEntry, exists: Option<bool>) -> Self {
        let path = std::path::Path::new(&entry.path);
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| entry.path.clone());
        let folder = path
            .parent()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
        Self {
            path: entry.path.clone(),
            name,
            folder,
            exists,
        }
    }
}

impl From<&RecentFileEntry> for RecentFileDto {
    fn from(entry: &RecentFileEntry) -> Self {
        let exists = Some(std::path::Path::new(&entry.path).exists());
        Self::from_entry(entry, exists)
    }
}

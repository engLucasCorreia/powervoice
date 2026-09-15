//! Module & rack preset DTOs (T-406, SPEC-012 §2.7, ADR-005 §2/§10).

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use vox_presets::PresetError;

use crate::ipc::error::{IpcError, IpcErrorCode};
use crate::ipc::rack_dto::LocalizedTextDto;

/// One entry of a module or rack preset menu (`module_presets_list`/`rack_presets_list`):
/// read-only factory presets first, then user-saved ones (both callers sort user names
/// alphabetically; factory order is the module's own, ADR-005 §2).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "bindings.ts")]
pub struct PresetEntryDto {
    /// Stable key (factory) or the sanitized file name (user) — pass back as [`PresetRefDto`].
    pub key: String,
    /// Display name.
    pub name: LocalizedTextDto,
    /// Read-only (`ModuleFactory::presets`/[`vox_presets::factory_rack_presets`]): no
    /// rename/delete menu item.
    pub is_factory: bool,
}

/// Which preset to load (`module_preset_load`/`rack_preset_load`) or manage
/// (`module_preset_rename`/`_delete`, `rack_preset_rename`/`_delete`): a factory preset (by its
/// stable key) or a user-saved one (by its stored name).
#[derive(Debug, Clone, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(export, export_to = "bindings.ts", rename_all = "snake_case")]
pub enum PresetRefDto {
    Factory { key: String },
    User { name: String },
}

/// One imported module preset (`module_preset_import`, H-22): which module it belongs to (the
/// file's own `module_id`, not necessarily whichever module tab the Manage Presets dialog had
/// open) plus the saved entry, so the UI can switch to — and refresh — the right list.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, export_to = "bindings.ts")]
pub struct ModulePresetImportedDto {
    pub module_id: String,
    pub entry: PresetEntryDto,
}

/// Maps a preset-storage failure to the shared IPC error shape (ADR-003). Every variant's
/// message is pre-rendered English text carrying the (already sanitized) preset name — like
/// [`crate::ipc::rack_dto::rack_ipc_error`]'s `Rack` variant, not something the UI has an i18n
/// key for. `AlreadyExists` gets its own key (plus the conflicting `name` as a structured param)
/// so a save/import can offer "Replace preset ‹name›?" instead of just reporting the failure
/// (H-22) — every other variant keeps the original generic key.
pub fn preset_ipc_error(err: PresetError) -> IpcError {
    let message = err.to_string();
    match err {
        PresetError::NotFound(_) => IpcError::new(IpcErrorCode::NotFound, "error.preset_rejected")
            .with_param("message", message),
        PresetError::AlreadyExists(name) => {
            IpcError::new(IpcErrorCode::InvalidArgument, "error.preset_already_exists")
                .with_param("message", message)
                .with_param("name", name)
        }
        PresetError::InvalidName(_) => {
            IpcError::new(IpcErrorCode::InvalidArgument, "error.preset_rejected")
                .with_param("message", message)
        }
        PresetError::Corrupt { .. } | PresetError::TooNew { .. } => {
            IpcError::new(IpcErrorCode::Internal, "error.preset_rejected")
                .with_param("message", message)
        }
        PresetError::Io(_) => {
            IpcError::new(IpcErrorCode::Io, "error.preset_rejected").with_param("message", message)
        }
    }
}

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

/// Maps a preset-storage failure to the shared IPC error shape (ADR-003). Every variant's
/// message is pre-rendered English text carrying the (already sanitized) preset name — like
/// [`crate::ipc::rack_dto::rack_ipc_error`]'s `Rack` variant, not something the UI has an i18n
/// key for.
pub fn preset_ipc_error(err: PresetError) -> IpcError {
    let code = match err {
        PresetError::NotFound(_) => IpcErrorCode::NotFound,
        PresetError::InvalidName(_) | PresetError::AlreadyExists(_) => {
            IpcErrorCode::InvalidArgument
        }
        PresetError::Corrupt { .. } | PresetError::TooNew { .. } => IpcErrorCode::Internal,
        PresetError::Io(_) => IpcErrorCode::Io,
    };
    IpcError::new(code, "error.preset_rejected").with_param("message", err.to_string())
}

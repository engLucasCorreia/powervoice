//! Tauri IPC layer (ADR-003): DTOs, the command registry, and the shared error type.
//!
//! This is the only module allowed to depend on `tauri` and `ts-rs` (ADR-001 rule 3). Domain
//! crates never see either; `src-tauri` maps domain types to these DTOs with `From` impls.

mod commands;
mod dto;
mod error;
mod macros;

// A glob import, not a named one: `#[tauri::command]` also generates a hidden macro alongside
// each command function (in the macro namespace), and `tauri::generate_handler!` below needs
// that hidden macro in scope, not just the function.
pub use commands::*;
pub use dto::AppInfo;
pub use error::{IpcError, IpcErrorCode};

// T-007 / ADR-009: the dev-only platform spike registers its commands here too (rather than
// duplicating `ipc_commands!`/`invoke_handler` machinery) only when built with `--features
// spike` (see `src-tauri/Cargo.toml`, `just spike`). A production build (`just dev`/`just
// build`/`just check` with default features) never sees `crate::spike` at all.
#[cfg(feature = "spike")]
pub use crate::spike::*;

#[cfg(not(feature = "spike"))]
crate::ipc_commands!(app_info);

#[cfg(feature = "spike")]
crate::ipc_commands!(
    app_info,
    spike_env,
    spike_waveform_peaks,
    spike_spectrogram_texture,
    spike_ipc_response_10mb,
    spike_ipc_channel_10mb,
    spike_telemetry_run,
    spike_write_results,
    spike_exit,
);

#[cfg(test)]
mod tests {
    use super::*;

    /// Guards `COMMAND_NAMES` (fed to `tauri::generate_handler!`) and the generated `CommandName`
    /// TS union against drifting apart (ADR-003: "a unit test asserts that the list equals the
    /// variants of the generated `CommandName` string union").
    #[test]
    fn command_names_match_command_name_variants() {
        let serialized: Vec<String> = COMMAND_NAME_VARIANTS
            .iter()
            .map(|v| {
                serde_json::to_string(v)
                    .unwrap()
                    .trim_matches('"')
                    .to_string()
            })
            .collect();
        let expected: Vec<String> = COMMAND_NAMES.iter().map(|s| s.to_string()).collect();
        assert_eq!(serialized, expected);
    }
}

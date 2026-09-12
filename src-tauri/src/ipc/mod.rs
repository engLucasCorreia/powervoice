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

crate::ipc_commands!(app_info);

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

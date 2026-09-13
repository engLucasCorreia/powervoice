//! Tauri IPC layer (ADR-003): DTOs, the command registry, events, and the shared error type.
//!
//! `tauri`/`ts-rs` usage is confined to `src-tauri` at the crate level (ADR-001 rule 3: "`tauri`
//! and `ts-rs`: only `src-tauri`"). Within this crate, this module plus the closely related
//! `settings` module (T-104: settings have no separate domain crate to keep those out of, so
//! their schema derives `ts_rs::TS` directly) are where DTOs live; domain crates never see
//! either dependency. `src-tauri` maps domain → DTO with `From` impls, which the compiler checks.

mod audio_commands;
mod audio_dto;
mod commands;
pub mod document_commands;
pub mod document_dto;
mod dto;
mod error;
mod events;
mod macros;

// A glob import, not a named one: `#[tauri::command]` also generates a hidden macro alongside
// each command function (in the macro namespace), and `tauri::generate_handler!` below needs
// that hidden macro in scope, not just the function.
pub use audio_commands::*;
pub use audio_dto::{
    DeviceDto, DeviceStatusDto, DevicesDto, TransportStateDto, notice_from_device,
};
pub use commands::*;
pub use document_commands::*;
pub use document_dto::{DocumentDto, PeaksRequestDto};
pub use dto::AppInfo;
pub use error::{IpcError, IpcErrorCode};
pub use events::{EVENT_NAME_VARIANTS, EVENT_NAMES, EventName, Notice, NoticeLevel, emit_notice};

// T-007 / ADR-009: the dev-only platform spike registers its commands here too (rather than
// duplicating `ipc_commands!`/`invoke_handler` machinery) only when built with `--features
// spike` (see `src-tauri/Cargo.toml`, `just spike`). A production build (`just dev`/`just
// build`/`just check` with default features) never sees `crate::spike` at all.
#[cfg(feature = "spike")]
pub use crate::spike::*;

#[cfg(not(feature = "spike"))]
crate::ipc_commands!(
    app_info,
    settings_get,
    settings_set,
    devices_list,
    devices_select,
    transport_get,
    transport_play,
    transport_pause,
    transport_stop,
    transport_play_from_start,
    transport_return_to_start,
    transport_seek,
    telemetry_subscribe,
    clock_now_ns,
    document_open,
    document_save,
    document_save_as,
    peaks_get,
);

#[cfg(feature = "spike")]
crate::ipc_commands!(
    app_info,
    settings_get,
    settings_set,
    devices_list,
    devices_select,
    transport_get,
    transport_play,
    transport_pause,
    transport_stop,
    transport_play_from_start,
    transport_return_to_start,
    transport_seek,
    telemetry_subscribe,
    clock_now_ns,
    document_open,
    document_save,
    document_save_as,
    peaks_get,
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

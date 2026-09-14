//! Tauri IPC layer (ADR-003): DTOs, the command registry, events, and the shared error type.
//!
//! `tauri`/`ts-rs` usage is confined to `src-tauri` at the crate level (ADR-001 rule 3: "`tauri`
//! and `ts-rs`: only `src-tauri`"). Within this crate, this module plus the closely related
//! `settings` module (T-104: settings have no separate domain crate to keep those out of, so
//! their schema derives `ts_rs::TS` directly) are where DTOs live; domain crates never see
//! either dependency. `src-tauri` maps domain → DTO with `From` impls, which the compiler checks.

mod acx_commands;
mod acx_dto;
mod analyzer_commands;
mod analyzer_dto;
mod audio_commands;
mod audio_dto;
mod calibration_commands;
mod commands;
pub mod document_commands;
pub mod document_dto;
mod dto;
mod error;
mod events;
pub mod export_commands;
pub mod export_dto;
pub mod loudness_commands;
pub mod loudness_dto;
mod macros;
mod normalize_commands;
mod normalize_dto;
mod nr_capture_commands;
mod nr_capture_dto;
mod preset_commands;
mod preset_dto;
mod rack_commands;
mod rack_dto;
mod recent_files_commands;
mod recent_files_dto;
mod record_commands;
mod record_dto;
mod record_op_dto;
mod recovery_commands;
mod recovery_dto;
mod spectro_commands;
mod spectro_dto;

// A glob import, not a named one: `#[tauri::command]` also generates a hidden macro alongside
// each command function (in the macro namespace), and `tauri::generate_handler!` below needs
// that hidden macro in scope, not just the function.
pub use acx_commands::*;
pub use acx_dto::{AcxCheckReportDto, AcxCheckRequestDto, AcxRuleDto, AcxRuleStatusDto};
pub use analyzer_commands::*;
pub use analyzer_dto::AnalyzerResponseDto;
pub use audio_commands::*;
pub use audio_dto::{
    DeviceDto, DeviceStatusDto, DevicesDto, TransportStateDto, notice_from_device,
};
pub use calibration_commands::*;
pub use commands::*;
pub use document_commands::*;
pub use document_dto::{
    ClipboardChangedDto, DocumentDto, DocumentProbeDto, EditResultDto, EditTargetDto,
    HistoryStateDto, MarkerDto, MarkerRangeKindDto, PeaksRequestDto, ProbeChannelDto,
    WaveformSelectionDto, WaveformViewDto,
};
pub use dto::AppInfo;
pub use error::{IpcError, IpcErrorCode};
pub use events::{
    EVENT_NAME_VARIANTS, EVENT_NAMES, EventName, ImportStartedDto, JobKind, JobProgressDto,
    JobState, Notice, NoticeLevel, emit_import_started, emit_job_progress, emit_loudness_report,
    emit_normalize_result, emit_notice,
};
pub use export_commands::*;
pub use export_dto::{
    ExportFormatDto, ExportFormatsDto, ExportRangeDto, ExportRequestDto, ExportStartedDto,
    Mp3SettingsDto,
};
pub use loudness_commands::*;
pub use loudness_dto::{
    LoudnessAnalyzeRequestDto, LoudnessAnalyzeStartedDto, LoudnessReportDto, LoudnessSourceDto,
};
pub use normalize_commands::*;
pub use normalize_dto::{NormalizeJobStartedDto, NormalizeResultDto};
pub use nr_capture_commands::*;
pub use nr_capture_dto::NrCaptureStartedDto;
pub use preset_commands::*;
pub use preset_dto::{PresetEntryDto, PresetRefDto};
pub use rack_commands::*;
pub use rack_dto::{
    NoiseProfileStatusDto, ParamChangedDto, RackLatencyDto, RackStateDto, rack_ipc_error,
};
pub use recent_files_commands::*;
pub use recent_files_dto::RecentFileDto;
pub use record_commands::*;
pub use record_dto::{RecordStateDto, engine_monitor_mode};
pub use record_op_dto::{
    CalibrationRejectDto, CalibrationResultDto, RecordCancelDto, RecordFinishedDto,
    RecordOffsetDto, RecordOpDto, RecordPhaseDto, RecordPhaseKindDto, RecordStartedDto,
};
pub use recovery_commands::*;
pub use recovery_dto::{
    RecoverResultDto, RecoverableSessionDto, RecoveredTakeActionDto, StorageInfoDto,
};
pub use spectro_commands::*;
pub use spectro_dto::SpectroRequestDto;

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
    rack_list_modules,
    rack_get,
    rack_add,
    rack_remove,
    rack_move,
    rack_bypass,
    rack_ab,
    rack_restart,
    param_set_normalized,
    param_set_text,
    param_set_plain,
    rack_response_curve,
    module_telemetry_subscribe,
    module_presets_list,
    module_preset_save,
    module_preset_load,
    module_preset_rename,
    module_preset_delete,
    module_reset_default,
    rack_presets_list,
    rack_preset_save,
    rack_preset_load,
    rack_preset_rename,
    rack_preset_delete,
    analyzer_subscribe,
    analyzer_set_response,
    analyzer_unsubscribe,
    record_get,
    record_arm,
    record_start,
    record_stop,
    record_set_monitor,
    record_peaks_get,
    document_open,
    document_open_cancel,
    document_probe,
    document_save,
    document_save_as,
    document_close,
    sidecar_view_set_spectral,
    sidecar_view_set_waveform,
    recent_files_get,
    recent_files_remove,
    recent_files_clear,
    recovery_list,
    recovery_recover,
    recovery_discard,
    storage_info,
    peaks_get,
    spectro_attach,
    spectro_detach,
    spectro_request,
    edit_cut,
    edit_copy,
    edit_paste,
    edit_delete,
    edit_trim,
    edit_silence,
    edit_normalize_peak_start,
    edit_normalize_peak_cancel,
    edit_normalize_lufs_start,
    edit_normalize_lufs_cancel,
    history_undo,
    history_redo,
    markers_get,
    marker_add,
    marker_rename,
    marker_set_range,
    marker_delete,
    export_formats,
    export_start,
    export_cancel,
    nr_capture_start,
    nr_capture_cancel,
    loudness_analyze_start,
    loudness_analyze_cancel,
    acx_check,
    record_start_at,
    record_offset_get,
    record_offset_set,
    calibration_run,
    calibration_cancel,
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
    rack_list_modules,
    rack_get,
    rack_add,
    rack_remove,
    rack_move,
    rack_bypass,
    rack_ab,
    rack_restart,
    param_set_normalized,
    param_set_text,
    param_set_plain,
    rack_response_curve,
    module_telemetry_subscribe,
    module_presets_list,
    module_preset_save,
    module_preset_load,
    module_preset_rename,
    module_preset_delete,
    module_reset_default,
    rack_presets_list,
    rack_preset_save,
    rack_preset_load,
    rack_preset_rename,
    rack_preset_delete,
    analyzer_subscribe,
    analyzer_set_response,
    analyzer_unsubscribe,
    record_get,
    record_arm,
    record_start,
    record_stop,
    record_set_monitor,
    record_peaks_get,
    document_open,
    document_open_cancel,
    document_probe,
    document_save,
    document_save_as,
    document_close,
    sidecar_view_set_spectral,
    sidecar_view_set_waveform,
    recent_files_get,
    recent_files_remove,
    recent_files_clear,
    recovery_list,
    recovery_recover,
    recovery_discard,
    storage_info,
    peaks_get,
    spectro_attach,
    spectro_detach,
    spectro_request,
    edit_cut,
    edit_copy,
    edit_paste,
    edit_delete,
    edit_trim,
    edit_silence,
    edit_normalize_peak_start,
    edit_normalize_peak_cancel,
    edit_normalize_lufs_start,
    edit_normalize_lufs_cancel,
    history_undo,
    history_redo,
    markers_get,
    marker_add,
    marker_rename,
    marker_set_range,
    marker_delete,
    export_formats,
    export_start,
    export_cancel,
    nr_capture_start,
    nr_capture_cancel,
    loudness_analyze_start,
    loudness_analyze_cancel,
    acx_check,
    record_start_at,
    record_offset_get,
    record_offset_set,
    calibration_run,
    calibration_cancel,
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

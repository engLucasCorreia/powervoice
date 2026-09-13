use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Error shape returned by every command (ADR-003). `key` is an i18n key, `params` fills its
/// placeholders (CLAUDE.md: every user-facing string goes through i18n — commands never return
/// pre-rendered text).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "bindings.ts")]
pub struct IpcError {
    pub code: IpcErrorCode,
    pub key: String,
    pub params: HashMap<String, String>,
}

impl std::fmt::Display for IpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} ({:?})", self.key, self.code)
    }
}

impl std::error::Error for IpcError {}

/// Coarse error classification shared by every command (ADR-003). This ticket (T-104) fixes the
/// taxonomy shape and fills in the codes already named by an approved spec/ADR; a command-owning
/// ticket adds a new variant here only when its own spec names a distinct error condition, to
/// keep this a small, meaningful set rather than one variant per command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "bindings.ts", rename_all = "snake_case")]
pub enum IpcErrorCode {
    /// Unexpected internal failure. Details go to the log (and `params.message` for debugging),
    /// never trusted as user-facing prose on its own — the UI still renders `key` through i18n.
    Internal,
    /// A command argument failed validation.
    InvalidArgument,
    /// The referenced document/file/session/etc. doesn't exist (any more).
    NotFound,
    /// SPEC-004 AC-15: an audio edit, undo or redo command is refused while a recording is in
    /// progress. i18n key: `error.not_while_recording`.
    NotWhileRecording,
    /// SPEC-001: a device named in a command/settings no longer exists.
    DeviceNotFound,
    /// SPEC-001 §2.4: the device was lost (disconnected/errored) while in use.
    DeviceLost,
    /// A filesystem operation failed (settings, sessions, exports, ...).
    Io,
    /// A long-running, cancellable operation was cancelled by the user (SPEC-004 §2.1).
    Cancelled,
    /// SPEC-010 §2.1: a command that changes the document is refused because another document
    /// job is already running (H-09: a normalize job). i18n key: `error.document_busy`.
    Busy,
}

impl IpcError {
    pub fn new(code: IpcErrorCode, key: impl Into<String>) -> Self {
        Self {
            code,
            key: key.into(),
            params: HashMap::new(),
        }
    }

    #[must_use]
    pub fn with_param(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.params.insert(name.into(), value.into());
        self
    }

    /// A generic internal error, carrying the underlying message as a `params.message` value for
    /// logs/debugging. The i18n string for `error.internal` itself never quotes it directly.
    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(IpcErrorCode::Internal, "error.internal").with_param("message", message.into())
    }

    /// SPEC-004 AC-15's refusal error, with its fixed i18n key.
    pub fn not_while_recording() -> Self {
        Self::new(IpcErrorCode::NotWhileRecording, "error.not_while_recording")
    }

    /// SPEC-010 §2.1's refusal error (H-09: another normalize job is already running).
    pub fn document_busy() -> Self {
        Self::new(IpcErrorCode::Busy, "error.document_busy")
    }
}

impl From<std::io::Error> for IpcError {
    fn from(err: std::io::Error) -> Self {
        Self::new(IpcErrorCode::Io, "error.io").with_param("message", err.to_string())
    }
}

impl From<crate::settings::SettingsError> for IpcError {
    fn from(err: crate::settings::SettingsError) -> Self {
        Self::new(IpcErrorCode::Io, "error.settings_io").with_param("message", err.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builders_populate_code_key_and_params() {
        let err = IpcError::new(IpcErrorCode::InvalidArgument, "error.invalid_argument")
            .with_param("field", "sample_rate_hz");
        assert_eq!(err.code, IpcErrorCode::InvalidArgument);
        assert_eq!(err.key, "error.invalid_argument");
        assert_eq!(
            err.params.get("field").map(String::as_str),
            Some("sample_rate_hz")
        );
    }

    #[test]
    fn not_while_recording_uses_the_spec_004_ac_15_key() {
        let err = IpcError::not_while_recording();
        assert_eq!(err.key, "error.not_while_recording");
        assert_eq!(err.code, IpcErrorCode::NotWhileRecording);
    }

    #[test]
    fn io_error_conversion_sets_the_io_code() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "nope");
        let err: IpcError = io_err.into();
        assert_eq!(err.code, IpcErrorCode::Io);
        assert_eq!(err.key, "error.io");
    }

    #[test]
    fn error_codes_serialize_snake_case() {
        let json = serde_json::to_string(&IpcErrorCode::NotWhileRecording).unwrap();
        assert_eq!(json, "\"not_while_recording\"");
        let json = serde_json::to_string(&IpcErrorCode::DeviceNotFound).unwrap();
        assert_eq!(json, "\"device_not_found\"");
    }
}

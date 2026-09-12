use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Error shape returned by every command (ADR-003). `key` is an i18n key, `params` fills its
/// placeholders. This is a stub: the full error taxonomy and code list land in T-104.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "bindings.ts")]
pub struct IpcError {
    pub code: IpcErrorCode,
    pub key: String,
    pub params: HashMap<String, String>,
}

impl std::fmt::Display for IpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.key)
    }
}

impl std::error::Error for IpcError {}

/// Coarse error classification. T-104 fills in the full set of codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "bindings.ts", rename_all = "snake_case")]
pub enum IpcErrorCode {
    Internal,
}

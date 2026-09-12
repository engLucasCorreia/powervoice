use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Basic app identity shown by the UI at startup. Returned by the `app_info` command.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "bindings.ts")]
pub struct AppInfo {
    pub name: String,
    pub version: String,
}

//! Plugin manager DTOs (T-804 item 6; the plugin manager UI itself is T-809).

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use vox_plugin_host::blocklist::BlockInfo;
use vox_plugin_host::health::CrashFlag;
use vox_plugin_host::{ClapPluginRef, SandboxSpec};

/// A plugin's status, as the plugin manager shows it (T-804 item 6). Only one applies at a time;
/// see [`crate::plugins::plugins_list`] for the precedence when more than one could.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(export, export_to = "bindings.ts", rename_all = "snake_case")]
pub enum PluginStatusDto {
    /// Working normally.
    Ok,
    /// Hidden from Add Module (`Settings.plugins.disabled`); still loads for a document that
    /// already uses it.
    Disabled,
    /// Blocklisted (ADR-008 §5): not in the registry at all until unblocked.
    Blocklisted { reason: String },
    /// Registered and working, but has crashed at runtime at least once (ADR-008 §5) — a
    /// warning badge, not a block; the user decides whether to block it.
    Flagged { crash_count: u32 },
}

/// One audio port side's channel count (T-804 item 3, ADR-008 §6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "bindings.ts")]
pub struct PluginPortsDto {
    pub input_channels: u32,
    pub output_channels: u32,
}

/// One plugin, for the plugin manager (T-804 item 6, T-809 UI).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "bindings.ts")]
pub struct PluginEntryDto {
    /// Registry module id (`"clap:<id>"`, …); empty for a blocklisted file that crashed before
    /// it could even report its id.
    pub id: String,
    pub name: String,
    pub vendor: String,
    pub version: String,
    /// Backend name (`"clap"`, …).
    pub format: String,
    /// The plugin file/bundle path.
    pub path: String,
    pub status: PluginStatusDto,
    /// `None` for a blocklisted entry (never instantiated to find out) or a format that doesn't
    /// report ports this way.
    pub ports: Option<PluginPortsDto>,
    pub param_count: u32,
}

/// `Settings.plugins.disabled`'s ids, as a fast lookup.
pub fn is_disabled(disabled: &[String], id: &str) -> bool {
    disabled.iter().any(|d| d == id)
}

/// A registered CLAP effect's plugin manager entry.
pub fn clap_entry(spec: &SandboxSpec, disabled: &[String], flag: CrashFlag) -> PluginEntryDto {
    let path = ClapPluginRef::parse(&spec.plugin)
        .map(|r| r.path)
        .unwrap_or_default();
    let status = if is_disabled(disabled, &spec.descriptor.id) {
        PluginStatusDto::Disabled
    } else if flag.crash_count > 0 {
        PluginStatusDto::Flagged {
            crash_count: flag.crash_count,
        }
    } else {
        PluginStatusDto::Ok
    };
    PluginEntryDto {
        id: spec.descriptor.id.clone(),
        name: spec.descriptor.name.text.clone(),
        vendor: spec.descriptor.vendor.clone(),
        version: spec.descriptor.version.to_string(),
        format: spec.format.clone(),
        path,
        status,
        ports: None,
        param_count: 0,
    }
}

/// A blocklisted file's plugin manager entry (its id/name/vendor are unknown — the scan crashed
/// or timed out before it could report a descriptor).
pub fn blocklisted_entry(path: &std::path::Path, info: &BlockInfo) -> PluginEntryDto {
    let name = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    PluginEntryDto {
        id: String::new(),
        name,
        vendor: String::new(),
        version: String::new(),
        format: "clap".to_string(),
        path: path.to_string_lossy().into_owned(),
        status: PluginStatusDto::Blocklisted {
            reason: info.reason.message().to_string(),
        },
        ports: None,
        param_count: 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vox_module_api::{LocalizedText, MODULE_API_VERSION, ModuleDescriptor, Version};
    use vox_plugin_host::blocklist::BlockReason;

    fn spec(id: &str) -> SandboxSpec {
        SandboxSpec {
            descriptor: ModuleDescriptor {
                id: id.to_string(),
                version: Version::new(1, 0, 0),
                name: LocalizedText::plain("De-esser"),
                vendor: "Acme".into(),
                description: LocalizedText::plain(""),
                url: None,
                features: vec!["audio-effect".into()],
                state_format_version: 1,
                api_version: MODULE_API_VERSION,
            },
            format: "clap".into(),
            plugin: ClapPluginRef {
                path: "/usr/lib/clap/acme.clap".into(),
                id: "com.acme.deesser".into(),
            }
            .to_reference(),
        }
    }

    #[test]
    fn ok_when_neither_disabled_nor_flagged() {
        let e = clap_entry(&spec("clap:com.acme.deesser"), &[], CrashFlag::default());
        assert_eq!(e.status, PluginStatusDto::Ok);
        assert_eq!(e.path, "/usr/lib/clap/acme.clap");
    }

    #[test]
    fn disabled_takes_priority_over_flagged() {
        let e = clap_entry(
            &spec("clap:com.acme.deesser"),
            &["clap:com.acme.deesser".to_string()],
            CrashFlag {
                crash_count: 3,
                last_unix_ms: 0,
            },
        );
        assert_eq!(e.status, PluginStatusDto::Disabled);
    }

    #[test]
    fn flagged_when_crashed_and_not_disabled() {
        let e = clap_entry(
            &spec("clap:com.acme.deesser"),
            &[],
            CrashFlag {
                crash_count: 2,
                last_unix_ms: 0,
            },
        );
        assert_eq!(e.status, PluginStatusDto::Flagged { crash_count: 2 });
    }

    #[test]
    fn blocklisted_entry_uses_the_file_stem_as_its_name() {
        let info = BlockInfo {
            reason: BlockReason::Crashed,
            blocked_at_unix_ms: 0,
        };
        let e = blocklisted_entry(std::path::Path::new("/tmp/bad-plugin.clap"), &info);
        assert_eq!(e.name, "bad-plugin");
        assert_eq!(
            e.status,
            PluginStatusDto::Blocklisted {
                reason: "crashed while being scanned".to_string()
            }
        );
    }
}

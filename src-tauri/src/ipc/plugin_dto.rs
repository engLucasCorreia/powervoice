//! Plugin manager DTOs (T-804 item 6; T-809: the plugin manager UI, "Install module…", folders).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use vox_plugin_host::blocklist::{BlockInfo, BlockReason};
use vox_plugin_host::health::CrashFlag;
use vox_plugin_host::install::InstallError;
use vox_plugin_host::scan::FailureKind;
use vox_plugin_host::{ClapPluginRef, InstallReport, PluginDetails, SandboxSpec};

/// Why a file is blocklisted (T-809: the UI words it through i18n; `reason` stays the log text).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "bindings.ts", rename_all = "snake_case")]
pub enum BlockCauseDto {
    /// The sandbox crashed while scanning it.
    Crashed,
    /// The scan didn't finish in time.
    TimedOut,
    /// The user blocked it.
    Manual,
}

impl From<BlockReason> for BlockCauseDto {
    fn from(r: BlockReason) -> Self {
        match r {
            BlockReason::Crashed => Self::Crashed,
            BlockReason::TimedOut => Self::TimedOut,
            BlockReason::Manual => Self::Manual,
        }
    }
}

/// A plugin's status, as the plugin manager shows it (T-804 item 6). Only one applies at a time;
/// see [`plugin_list`] for the precedence when more than one could.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(export, export_to = "bindings.ts", rename_all = "snake_case")]
pub enum PluginStatusDto {
    /// Working normally.
    Ok,
    /// Hidden from Add Module (`Settings.plugins.disabled`); still loads for a document that
    /// already uses it.
    Disabled,
    /// Blocklisted (ADR-008 §5): skipped by scans until unblocked or changed. `reason` is the
    /// English log text; `cause` is what the UI words.
    Blocklisted {
        reason: String,
        cause: BlockCauseDto,
    },
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

fn blocked_status(info: &BlockInfo) -> PluginStatusDto {
    PluginStatusDto::Blocklisted {
        reason: info.reason.message().to_string(),
        cause: info.reason.into(),
    }
}

/// A registered CLAP effect's plugin manager entry. `details`: what the scan learned (ports and
/// parameter count; T-809 — unknown ports stay `None`).
pub fn clap_entry(
    spec: &SandboxSpec,
    disabled: &[String],
    flag: CrashFlag,
    details: Option<PluginDetails>,
) -> PluginEntryDto {
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
    let details = details.unwrap_or_default();
    let ports =
        (details.input_channels > 0 || details.output_channels > 0).then_some(PluginPortsDto {
            input_channels: details.input_channels,
            output_channels: details.output_channels,
        });
    PluginEntryDto {
        id: spec.descriptor.id.clone(),
        name: spec.descriptor.name.text.clone(),
        vendor: spec.descriptor.vendor.clone(),
        version: spec.descriptor.version.to_string(),
        format: spec.format.clone(),
        path,
        status,
        ports,
        param_count: details.param_count,
    }
}

/// A blocklisted file's plugin manager entry (its id/name/vendor are unknown — the scan crashed
/// or timed out before it could report a descriptor).
pub fn blocklisted_entry(path: &Path, info: &BlockInfo) -> PluginEntryDto {
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
        status: blocked_status(info),
        ports: None,
        param_count: 0,
    }
}

/// `plugins_list`'s answer: every registered effect (sorted by id; `Disabled` before `Flagged`
/// before `Ok`), except that one whose **file** is blocklisted shows as `Blocklisted` — keeping
/// its real name — (T-809: blocking a registered plugin, or a flagged one, until the next
/// rescan drops it); then every other blocklisted file, by path, identified by its file name.
pub fn plugin_list(
    specs: &[SandboxSpec],
    details: impl Fn(&str) -> Option<PluginDetails>,
    disabled: &[String],
    flag: impl Fn(&str) -> CrashFlag,
    blocked: &[(PathBuf, BlockInfo)],
) -> Vec<PluginEntryDto> {
    let blocked_info = |path: &str| {
        blocked
            .iter()
            .find(|(p, _)| p.to_string_lossy() == path)
            .map(|(_, info)| info)
    };
    let mut list: Vec<PluginEntryDto> = specs
        .iter()
        .map(|spec| {
            let id = &spec.descriptor.id;
            let mut entry = clap_entry(spec, disabled, flag(id), details(id));
            if let Some(info) = blocked_info(&entry.path) {
                entry.status = blocked_status(info);
            }
            entry
        })
        .collect();
    list.sort_by(|a, b| a.id.cmp(&b.id));
    for (path, info) in blocked {
        let key = path.to_string_lossy();
        if !list.iter().any(|e| e.path == key) {
            list.push(blocklisted_entry(path, info));
        }
    }
    list
}

/// One effect an install added (T-809).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "bindings.ts")]
pub struct InstalledEffectDto {
    pub id: String,
    pub name: String,
}

/// Why "Install module…" didn't install (T-809). The UI words each one; `detail` carries the
/// underlying (English, OS- or plugin-provided) text for `scan_failed`/`io`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "bindings.ts", rename_all = "snake_case")]
pub enum InstallFailureDto {
    NotFound,
    NotAPlugin,
    AlreadyInstalled,
    Blocklisted,
    NoEffects,
    ScanCrashed,
    ScanTimedOut,
    ScanFailed,
    NoInstallDir,
    Io,
}

/// `plugins_install`'s answer (T-809).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(export, export_to = "bindings.ts", rename_all = "snake_case")]
pub enum PluginInstallResultDto {
    /// Copied into `path` (the per-user plugin folder) and registered; in Add Module now.
    Installed {
        path: String,
        replaced: bool,
        effects: Vec<InstalledEffectDto>,
    },
    /// A plugin with the same file name is installed at `path`; nothing changed. Ask, then call
    /// again with `replace: true`.
    Collision { path: String },
    /// Nothing was installed (a replaced file is restored). `blocklisted`: the picked file was
    /// blocklisted (the scan crashed or timed out); `cause` says why it is/was blocklisted.
    Failed {
        code: InstallFailureDto,
        detail: String,
        blocklisted: bool,
        cause: Option<BlockCauseDto>,
    },
}

impl From<Result<InstallReport, InstallError>> for PluginInstallResultDto {
    fn from(result: Result<InstallReport, InstallError>) -> Self {
        let err = match result {
            Ok(report) => {
                return Self::Installed {
                    path: report.target.to_string_lossy().into_owned(),
                    replaced: report.replaced,
                    effects: report
                        .effects
                        .iter()
                        .map(|s| InstalledEffectDto {
                            id: s.descriptor.id.clone(),
                            name: s.descriptor.name.text.clone(),
                        })
                        .collect(),
                };
            }
            Err(InstallError::Collision { target }) => {
                return Self::Collision {
                    path: target.to_string_lossy().into_owned(),
                };
            }
            Err(e) => e,
        };
        let detail = err.to_string();
        let (code, blocklisted, cause) = match err {
            InstallError::NotFound => (InstallFailureDto::NotFound, false, None),
            InstallError::NotAPlugin => (InstallFailureDto::NotAPlugin, false, None),
            InstallError::AlreadyInstalled => (InstallFailureDto::AlreadyInstalled, false, None),
            InstallError::Blocklisted(reason) => {
                (InstallFailureDto::Blocklisted, true, Some(reason.into()))
            }
            InstallError::NoEffects => (InstallFailureDto::NoEffects, false, None),
            InstallError::ScanFailed {
                kind, blocklisted, ..
            } => match kind {
                FailureKind::Crashed => (
                    InstallFailureDto::ScanCrashed,
                    blocklisted,
                    Some(BlockCauseDto::Crashed),
                ),
                FailureKind::TimedOut => (
                    InstallFailureDto::ScanTimedOut,
                    blocklisted,
                    Some(BlockCauseDto::TimedOut),
                ),
                FailureKind::Other => (InstallFailureDto::ScanFailed, false, None),
            },
            InstallError::NoInstallDir => (InstallFailureDto::NoInstallDir, false, None),
            InstallError::Io(_) | InstallError::Collision { .. } => {
                (InstallFailureDto::Io, false, None)
            }
        };
        Self::Failed {
            code,
            detail,
            blocklisted,
            cause,
        }
    }
}

/// The folders PowerVoice scans, for the plugin manager's Folders tab and Preferences (T-809).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "bindings.ts")]
pub struct PluginFoldersDto {
    /// Where "Install module…" copies plugins (the per-user CLAP folder); `None` if the
    /// environment names none.
    pub install: Option<String>,
    /// The standard per-format folders (`$CLAP_PATH` first), always scanned.
    pub standard: Vec<String>,
    /// `Settings.plugins.custom_folders`.
    pub custom: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use vox_module_api::{LocalizedText, MODULE_API_VERSION, ModuleDescriptor, Version};

    fn spec(id: &str, path: &str) -> SandboxSpec {
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
                path: path.into(),
                id: "com.acme.deesser".into(),
            }
            .to_reference(),
        }
    }

    fn block(reason: BlockReason) -> BlockInfo {
        BlockInfo {
            reason,
            blocked_at_unix_ms: 0,
        }
    }

    const PATH: &str = "/usr/lib/clap/acme.clap";

    #[test]
    fn ok_when_neither_disabled_nor_flagged() {
        let e = clap_entry(
            &spec("clap:com.acme.deesser", PATH),
            &[],
            CrashFlag::default(),
            None,
        );
        assert_eq!(e.status, PluginStatusDto::Ok);
        assert_eq!(e.path, PATH);
        assert_eq!(e.ports, None, "never instantiated: unknown");
    }

    #[test]
    fn ports_and_params_come_from_the_scan_details() {
        let e = clap_entry(
            &spec("clap:com.acme.deesser", PATH),
            &[],
            CrashFlag::default(),
            Some(PluginDetails {
                param_count: 12,
                input_channels: 1,
                output_channels: 2,
            }),
        );
        assert_eq!(
            e.ports,
            Some(PluginPortsDto {
                input_channels: 1,
                output_channels: 2
            })
        );
        assert_eq!(e.param_count, 12);
    }

    #[test]
    fn disabled_takes_priority_over_flagged() {
        let e = clap_entry(
            &spec("clap:com.acme.deesser", PATH),
            &["clap:com.acme.deesser".to_string()],
            CrashFlag {
                crash_count: 3,
                last_unix_ms: 0,
            },
            None,
        );
        assert_eq!(e.status, PluginStatusDto::Disabled);
    }

    #[test]
    fn flagged_when_crashed_and_not_disabled() {
        let e = clap_entry(
            &spec("clap:com.acme.deesser", PATH),
            &[],
            CrashFlag {
                crash_count: 2,
                last_unix_ms: 0,
            },
            None,
        );
        assert_eq!(e.status, PluginStatusDto::Flagged { crash_count: 2 });
    }

    #[test]
    fn blocklisted_entry_uses_the_file_stem_as_its_name() {
        let e = blocklisted_entry(
            std::path::Path::new("/tmp/bad-plugin.clap"),
            &block(BlockReason::Crashed),
        );
        assert_eq!(e.name, "bad-plugin");
        assert_eq!(
            e.status,
            PluginStatusDto::Blocklisted {
                reason: "crashed while being scanned".to_string(),
                cause: BlockCauseDto::Crashed,
            }
        );
    }

    /// T-809: a registered plugin whose file gets blocked shows once, as blocklisted, with its
    /// real name — not twice (registered + a nameless blocklist row).
    #[test]
    fn a_blocked_registered_plugin_is_listed_once_with_its_name() {
        let blocked = vec![
            (PathBuf::from(PATH), block(BlockReason::Manual)),
            (
                PathBuf::from("/home/u/.clap/bad.clap"),
                block(BlockReason::TimedOut),
            ),
        ];
        let list = plugin_list(
            &[spec("clap:com.acme.deesser", PATH)],
            |_| None,
            &[],
            |_| CrashFlag::default(),
            &blocked,
        );
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].name, "De-esser");
        assert_eq!(
            list[0].status,
            PluginStatusDto::Blocklisted {
                reason: "blocked by the user".into(),
                cause: BlockCauseDto::Manual
            }
        );
        assert_eq!(list[1].name, "bad");
        assert!(matches!(
            list[1].status,
            PluginStatusDto::Blocklisted {
                cause: BlockCauseDto::TimedOut,
                ..
            }
        ));
    }

    #[test]
    fn install_results_map_to_codes() {
        let collision: PluginInstallResultDto = Err(InstallError::Collision {
            target: PathBuf::from("/home/u/.clap/a.clap"),
        })
        .into();
        assert_eq!(
            collision,
            PluginInstallResultDto::Collision {
                path: "/home/u/.clap/a.clap".into()
            }
        );
        let crashed: PluginInstallResultDto = Err(InstallError::ScanFailed {
            message: "crashed while being scanned (signal 11)".into(),
            kind: FailureKind::Crashed,
            blocklisted: true,
        })
        .into();
        assert_eq!(
            crashed,
            PluginInstallResultDto::Failed {
                code: InstallFailureDto::ScanCrashed,
                detail: "crashed while being scanned (signal 11)".into(),
                blocklisted: true,
                cause: Some(BlockCauseDto::Crashed),
            }
        );
        let ok: PluginInstallResultDto = Ok(InstallReport {
            target: PathBuf::from("/home/u/.clap/acme.clap"),
            effects: vec![spec("clap:com.acme.deesser", "/home/u/.clap/acme.clap")],
            replaced: false,
        })
        .into();
        assert_eq!(
            ok,
            PluginInstallResultDto::Installed {
                path: "/home/u/.clap/acme.clap".into(),
                replaced: false,
                effects: vec![InstalledEffectDto {
                    id: "clap:com.acme.deesser".into(),
                    name: "De-esser".into()
                }],
            }
        );
    }
}

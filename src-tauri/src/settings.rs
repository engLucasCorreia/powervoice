//! Versioned settings file (T-104): device prefs, default recording format, monitor mode,
//! telemetry rate and memory budget (SPEC-001 §2.5, SPEC-002 §3, SPEC-003 §3, SPEC-004 §2.4).
//!
//! The file lives in the OS config dir (SPEC-001 §2.5: "the app settings file managed by
//! `src-tauri`") and is written atomically (temp file + rename). Schema types derive `ts_rs::TS`
//! directly here rather than through a separate DTO layer: unlike device/transport/document
//! state, settings have no domain crate of their own to keep `tauri`/`ts-rs` out of (ADR-001 rule
//! 3 is a crate-level rule — "`tauri` and `ts-rs`: only `src-tauri`" — and this module still
//! keeps `tauri` itself out; only `src-tauri/src/ipc/commands.rs` touches `tauri::State`).
//!
//! TODO(T-108): `DevicePrefs` maps host/input/output device choices to the engine's own
//! `DevicePrefs` type (T-102, `crates/engine`). That type doesn't exist yet in this worktree, so
//! this module declares its own [`DevicePrefsDto`] with the fields SPEC-001 §2.5 requires
//! ("saved intent", not applied value) and leaves the engine <-> DTO mapping for whichever ticket
//! wires the two together.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Current on-disk schema version. Bump when a breaking shape change needs a real migration
/// (see [`migrate_to_current`]); non-breaking additions just need a new field with a `Default`.
pub const CURRENT_SETTINGS_VERSION: u32 = 1;

// --- Device prefs (SPEC-001 §2.5) --------------------------------------------------------------

/// Persisted device selection — "saved intent", not the applied value: SPEC-001 §2.5 requires
/// that a value which triggered a runtime fallback (unsupported sample rate, out-of-range buffer
/// size) is still saved as the user's *intent*, so PowerVoice retries it next time the device's
/// capabilities might have changed. This DTO therefore never carries a "what was actually
/// applied" field — that belongs to runtime state/telemetry (a later ticket), not settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "bindings.ts")]
pub struct DevicePrefsDto {
    /// Audio host/backend id (SPEC-001 §3: `alsa`|`pipewire`|`jack` on Linux, `wasapi` on
    /// Windows, `coreaudio` on macOS). Kept as a plain string rather than a closed enum: only
    /// the engine (T-102) can enumerate which hosts actually exist on this machine.
    pub host: String,
    /// `None` = "None" (SPEC-001 §3 factory default; disarms recording/monitoring input).
    /// Otherwise the device's host-reported name — devices are keyed by `(host, name)`, not
    /// index (SPEC-001 §2.2: cpal indices aren't stable across enumerations).
    pub input_device: Option<String>,
    /// 1-based input channel index (SPEC-001 §3: `1..=N`, N = device max input channels).
    pub input_channel: u32,
    /// `None` = host default output device (SPEC-001 §3).
    pub output_device: Option<String>,
    /// `None` = device default sample rate ("Auto" is not a documented option for rate in
    /// SPEC-001 §3, but no saved preference yet behaves the same as "use the device default").
    pub sample_rate_hz: Option<u32>,
    /// `None` = "Auto" (device default; SPEC-001 §3 factory default).
    pub buffer_size_frames: Option<u32>,
}

/// Host default per SPEC-001 §3 (Linux: PipeWire if available else ALSA; Windows: WASAPI;
/// macOS: CoreAudio). This is a *settings-file* seed only — it doesn't probe hardware (that's
/// the engine's job, T-102/T-108); it just picks the platform's usual first choice so a
/// brand-new settings file has a plausible default instead of an empty string.
pub fn default_host() -> String {
    if cfg!(target_os = "windows") {
        "wasapi"
    } else if cfg!(target_os = "macos") {
        "coreaudio"
    } else {
        // TODO(T-102/T-108): probe cpal for a reachable PipeWire server and fall back to "alsa"
        // (SPEC-001 §3) instead of always assuming PipeWire is available.
        "pipewire"
    }
    .to_string()
}

impl Default for DevicePrefsDto {
    fn default() -> Self {
        Self {
            host: default_host(),
            input_device: None,
            input_channel: 1,
            output_device: None,
            sample_rate_hz: None,
            buffer_size_frames: None,
        }
    }
}

// --- Default recording format (SPEC-002 §3) ----------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "bindings.ts")]
pub enum BitDepth {
    #[serde(rename = "16")]
    #[ts(rename = "16")]
    Bit16,
    #[serde(rename = "24")]
    #[ts(rename = "24")]
    Bit24,
    #[serde(rename = "32f")]
    #[ts(rename = "32f")]
    Bit32Float,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "bindings.ts")]
pub struct DefaultFormatDto {
    pub sample_rate_hz: u32,
    pub bit_depth: BitDepth,
}

impl Default for DefaultFormatDto {
    fn default() -> Self {
        // PROMPT §2 (LOCKED) / SPEC-002 §3 factory default: 48 kHz / 24-bit.
        Self {
            sample_rate_hz: 48_000,
            bit_depth: BitDepth::Bit24,
        }
    }
}

// --- Monitor mode (SPEC-002 §2.7, §3) -----------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "bindings.ts", rename_all = "snake_case")]
pub enum MonitorMode {
    // PROMPT §2 (LOCKED) / SPEC-002 §3 factory default.
    #[default]
    Off,
    Dry,
    ThroughRack,
}

// --- Normalize dialog memory (SPEC-010 §2.4, H-09) ----------------------------------------------

/// The Normalize… dialog's unit toggle (SPEC-010 §2.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "bindings.ts", rename_all = "snake_case")]
pub enum NormalizeTargetUnit {
    #[default]
    Db,
    Pct,
}

/// "The last applied value and unit are remembered in settings across restarts" (SPEC-010 §2.4).
/// `value` is in whichever unit it was last applied in — switching units in the dialog itself
/// converts the *shown* value without touching this until Apply.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "bindings.ts")]
pub struct NormalizeDialogPrefsDto {
    pub value: f64,
    pub unit: NormalizeTargetUnit,
}

impl Default for NormalizeDialogPrefsDto {
    fn default() -> Self {
        // SPEC-010 §2.4: "the default is −1.00 dB".
        Self {
            value: -1.0,
            unit: NormalizeTargetUnit::Db,
        }
    }
}

// --- RAM detection for the memory budget default (SPEC-004 §2.4, §3) ---------------------------

/// Parses `MemTotal:` (kiB) out of `/proc/meminfo` text. Pure function so it's unit-testable
/// without touching the real filesystem.
fn parse_mem_total_kib(meminfo: &str) -> Option<u64> {
    meminfo.lines().find_map(|line| {
        let mut parts = line.split_whitespace();
        if parts.next()? != "MemTotal:" {
            return None;
        }
        parts.next()?.parse::<u64>().ok()
    })
}

/// Total system RAM in bytes. Linux reads `/proc/meminfo`; other platforms have no portable
/// query available without a new dependency (`sysinfo` is not in this ticket's allowed-deps
/// list — `tracing`, `tracing-subscriber`, `tracing-appender`, `directories` only), so they fall
/// back to a conservative assumption. This mirrors the project's existing "Windows/macOS
/// unverified" risk (MEMORY.md) rather than introducing a new one.
pub fn total_ram_bytes() -> u64 {
    #[cfg(target_os = "linux")]
    {
        if let Ok(contents) = std::fs::read_to_string("/proc/meminfo")
            && let Some(kib) = parse_mem_total_kib(&contents)
        {
            return kib * 1024;
        }
    }
    8 * 1024 * 1024 * 1024 // 8 GiB fallback
}

const MIB: u64 = 1024 * 1024;

/// `clamp(RAM/4, 512 MiB, 4 GiB)` (SPEC-004 §2.4, §3 `memory_budget` default). The *setting*
/// itself allows up to 16 GiB (SPEC-004 §2.4), but the factory default clamps at 4 GiB.
pub fn default_memory_budget_mib(total_ram_bytes: u64) -> u32 {
    let quarter_mib = (total_ram_bytes / 4) / MIB;
    quarter_mib.clamp(512, 4096) as u32
}

// --- Settings root -------------------------------------------------------------------------------

/// The whole settings file. `#[serde(default)]` at the container level means any field missing
/// from the on-disk JSON (an older/partial file) is filled from [`Settings::default`], and the
/// flattened `extra` map preserves any *unknown* field a future version wrote, so round-tripping
/// through this version never silently drops it (acceptance criterion: "unknown future fields
/// preserved").
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(default)]
#[ts(export, export_to = "bindings.ts")]
pub struct Settings {
    pub version: u32,
    pub device: DevicePrefsDto,
    pub default_format: DefaultFormatDto,
    pub monitor_mode: MonitorMode,
    /// T-107 (SPEC-002 §2.7): the one-time "use headphones" hint was shown (it appears the first
    /// time monitoring is enabled).
    pub monitor_hint_shown: bool,
    /// SPEC-003 §3: {30, 60} Hz, default 60 (measured free on WebKitGTK, ADR-009 §3).
    pub telemetry_rate_hz: u32,
    pub memory_budget_mib: u32,
    /// H-09/SPEC-010 §2.4: the Normalize… dialog's last applied value and unit.
    pub normalize_dialog: NormalizeDialogPrefsDto,
    #[serde(flatten)]
    #[ts(skip)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            version: CURRENT_SETTINGS_VERSION,
            device: DevicePrefsDto::default(),
            default_format: DefaultFormatDto::default(),
            monitor_mode: MonitorMode::default(),
            monitor_hint_shown: false,
            telemetry_rate_hz: 60,
            memory_budget_mib: default_memory_budget_mib(total_ram_bytes()),
            normalize_dialog: NormalizeDialogPrefsDto::default(),
            extra: serde_json::Map::new(),
        }
    }
}

// --- Load / migrate / save -----------------------------------------------------------------------

#[derive(Debug)]
pub enum SettingsError {
    Io(std::io::Error),
    Json(serde_json::Error),
}

impl std::fmt::Display for SettingsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SettingsError::Io(e) => write!(f, "settings I/O error: {e}"),
            SettingsError::Json(e) => write!(f, "settings JSON error: {e}"),
        }
    }
}

impl std::error::Error for SettingsError {}

impl From<std::io::Error> for SettingsError {
    fn from(err: std::io::Error) -> Self {
        SettingsError::Io(err)
    }
}

impl From<serde_json::Error> for SettingsError {
    fn from(err: serde_json::Error) -> Self {
        SettingsError::Json(err)
    }
}

/// Migrates an arbitrary on-disk JSON value to the current version. Version 0 (no `version`
/// field at all, or `version: 0`) predates this schema entirely — no PowerVoice build ever wrote
/// a settings file before this ticket — so migrating it is just a version-stamp bump: `Settings`'s
/// container-level `#[serde(default)]` already fills in every field a v0 shape doesn't have.
/// Future migrations (v1 -> v2, ...) should transform old field shapes here *before* bumping the
/// stamp, the same way this function would gain a `1 => { ... }` arm.
fn migrate_to_current(mut value: serde_json::Value) -> serde_json::Value {
    if let serde_json::Value::Object(map) = &mut value {
        map.insert(
            "version".to_string(),
            serde_json::json!(CURRENT_SETTINGS_VERSION),
        );
    }
    value
}

/// Parses settings JSON, migrating it to [`CURRENT_SETTINGS_VERSION`] first if it's older.
pub fn parse_and_migrate(bytes: &[u8]) -> Result<Settings, SettingsError> {
    let mut value: serde_json::Value = serde_json::from_slice(bytes)?;
    let file_version = value
        .get("version")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    if file_version < u64::from(CURRENT_SETTINGS_VERSION) {
        value = migrate_to_current(value);
    }
    Ok(serde_json::from_value(value)?)
}

fn project_dirs() -> Option<directories::ProjectDirs> {
    directories::ProjectDirs::from("app", "powervoice", "powervoice")
}

/// The settings file path: `<OS config dir>/settings.json` (SPEC-001 §2.5).
pub fn settings_path() -> PathBuf {
    project_dirs()
        .map(|dirs| dirs.config_dir().join("settings.json"))
        .unwrap_or_else(|| {
            std::env::temp_dir()
                .join("powervoice")
                .join("settings.json")
        })
}

/// Atomic write: write to a temp file in the same directory, `fsync`, then `rename` over the
/// target (rename is atomic on the same filesystem on every platform this project ships for).
pub fn save(path: &Path, settings: &Settings) -> Result<(), SettingsError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_vec_pretty(settings)?;
    let tmp_name = format!(
        ".{}.tmp-{}",
        path.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("settings.json"),
        std::process::id()
    );
    let tmp_path = path.with_file_name(tmp_name);
    {
        let mut file = std::fs::File::create(&tmp_path)?;
        file.write_all(&json)?;
        file.sync_all()?;
    }
    std::fs::rename(&tmp_path, path)?;
    Ok(())
}

/// Loads settings from `path`, falling back to [`Settings::default`] when the file doesn't
/// exist or fails to parse (logged, never a panic/crash).
pub fn load_or_default(path: &Path) -> Settings {
    match std::fs::read(path) {
        Ok(bytes) => parse_and_migrate(&bytes).unwrap_or_else(|err| {
            tracing::warn!(error = %err, path = %path.display(), "settings file unreadable; using defaults");
            Settings::default()
        }),
        Err(_) => Settings::default(),
    }
}

/// In-memory settings cache backing the `settings_get`/`settings_set` commands, managed as Tauri
/// state. Reads never touch disk; writes save (atomically) before updating the cache.
pub struct SettingsStore {
    path: PathBuf,
    state: Mutex<Settings>,
}

impl SettingsStore {
    pub fn load_default() -> Self {
        let path = settings_path();
        let settings = load_or_default(&path);
        Self {
            path,
            state: Mutex::new(settings),
        }
    }

    #[cfg(test)]
    fn at_path(path: PathBuf) -> Self {
        let settings = load_or_default(&path);
        Self {
            path,
            state: Mutex::new(settings),
        }
    }

    pub fn get(&self) -> Settings {
        self.state.lock().expect("settings mutex poisoned").clone()
    }

    pub fn set(&self, mut settings: Settings) -> Result<Settings, SettingsError> {
        settings.version = CURRENT_SETTINGS_VERSION;
        save(&self.path, &settings)?;
        *self.state.lock().expect("settings mutex poisoned") = settings.clone();
        Ok(settings)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A unique, freshly-created temp directory (no `tempfile` dep: not in this ticket's
    /// allowed-deps list).
    fn temp_dir(label: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "powervoice-settings-test-{label}-{}-{nanos}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    #[allow(clippy::float_cmp)] // exact default value
    fn defaults_on_first_run() {
        let dir = temp_dir("first-run");
        let path = dir.join("settings.json");
        let settings = load_or_default(&path);

        assert_eq!(settings.version, CURRENT_SETTINGS_VERSION);
        assert_eq!(settings.default_format.sample_rate_hz, 48_000);
        assert_eq!(settings.default_format.bit_depth, BitDepth::Bit24);
        assert_eq!(settings.monitor_mode, MonitorMode::Off);
        assert_eq!(settings.telemetry_rate_hz, 60);
        assert_eq!(settings.device.input_channel, 1);
        assert_eq!(settings.device.input_device, None);
        assert_eq!(settings.device.output_device, None);
        assert_eq!(settings.device.sample_rate_hz, None);
        assert_eq!(settings.device.buffer_size_frames, None);
        assert!((512..=4096).contains(&settings.memory_budget_mib));
        assert_eq!(settings.normalize_dialog.value, -1.0);
        assert_eq!(settings.normalize_dialog.unit, NormalizeTargetUnit::Db);

        std::fs::remove_dir_all(&dir).ok();
    }

    /// H-09/SPEC-010 §2.4: "the last applied value and unit are remembered ... across restarts".
    #[test]
    #[allow(clippy::float_cmp)] // exact round-tripped value
    fn normalize_dialog_prefs_round_trip_through_save_and_load() {
        let dir = temp_dir("normalize-dialog");
        let path = dir.join("settings.json");

        let settings = Settings {
            normalize_dialog: NormalizeDialogPrefsDto {
                value: 89.1,
                unit: NormalizeTargetUnit::Pct,
            },
            ..Settings::default()
        };
        save(&path, &settings).unwrap();
        let loaded = load_or_default(&path);

        assert_eq!(loaded.normalize_dialog.value, 89.1);
        assert_eq!(loaded.normalize_dialog.unit, NormalizeTargetUnit::Pct);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn round_trips_through_save_and_load() {
        let dir = temp_dir("round-trip");
        let path = dir.join("settings.json");

        let mut settings = Settings::default();
        settings.device.host = "pipewire".to_string();
        settings.device.input_device = Some("Scarlett 2i2".to_string());
        settings.device.input_channel = 2;
        settings.monitor_mode = MonitorMode::ThroughRack;
        settings.telemetry_rate_hz = 30;

        save(&path, &settings).unwrap();
        let loaded = load_or_default(&path);

        assert_eq!(loaded, settings);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn atomic_write_leaves_no_temp_file_behind() {
        let dir = temp_dir("atomic");
        let path = dir.join("settings.json");
        save(&path, &Settings::default()).unwrap();

        let entries: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .collect();
        assert_eq!(
            entries.len(),
            1,
            "expected exactly one file (no leftover temp file)"
        );
        assert_eq!(entries[0].file_name(), "settings.json");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn unknown_future_fields_are_preserved_across_a_save() {
        let dir = temp_dir("future-fields");
        let path = dir.join("settings.json");

        // Simulate a file written by a hypothetical future version with a field this version
        // doesn't know about.
        let mut value = serde_json::to_value(Settings::default()).unwrap();
        value
            .as_object_mut()
            .unwrap()
            .insert("future_thing".to_string(), serde_json::json!({ "wow": 42 }));
        std::fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();

        let mut loaded = load_or_default(&path);
        assert_eq!(
            loaded.extra.get("future_thing"),
            Some(&serde_json::json!({ "wow": 42 }))
        );

        // Re-saving (as if the user changed some other setting) must not drop it.
        loaded.telemetry_rate_hz = 30;
        save(&path, &loaded).unwrap();
        let raw = std::fs::read_to_string(&path).unwrap();
        let raw_value: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(
            raw_value.get("future_thing"),
            Some(&serde_json::json!({ "wow": 42 }))
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn migrates_v0_file_with_no_version_field() {
        let json = br#"{"monitor_mode":"dry","telemetry_rate_hz":30}"#;
        let settings = parse_and_migrate(json).unwrap();

        assert_eq!(settings.version, CURRENT_SETTINGS_VERSION);
        assert_eq!(settings.monitor_mode, MonitorMode::Dry);
        assert_eq!(settings.telemetry_rate_hz, 30);
        // Fields the v0 shape never had fall back to the current defaults.
        assert_eq!(settings.default_format.sample_rate_hz, 48_000);
    }

    #[test]
    fn migrates_v0_file_with_explicit_version_zero() {
        let json = br#"{"version":0,"device":{"host":"alsa","input_device":null,"input_channel":1,"output_device":null,"sample_rate_hz":null,"buffer_size_frames":null}}"#;
        let settings = parse_and_migrate(json).unwrap();
        assert_eq!(settings.version, CURRENT_SETTINGS_VERSION);
        assert_eq!(settings.device.host, "alsa");
    }

    #[test]
    fn settings_store_get_set_round_trip() {
        let dir = temp_dir("store");
        let path = dir.join("settings.json");
        let store = SettingsStore::at_path(path.clone());

        let mut next = store.get();
        next.monitor_mode = MonitorMode::Dry;
        let saved = store.set(next.clone()).unwrap();
        assert_eq!(saved.monitor_mode, MonitorMode::Dry);
        assert_eq!(store.get().monitor_mode, MonitorMode::Dry);

        // Reloading from disk (a fresh store) sees the same value.
        let reloaded = SettingsStore::at_path(path);
        assert_eq!(reloaded.get().monitor_mode, MonitorMode::Dry);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn parse_mem_total_kib_reads_proc_meminfo_shape() {
        let sample = "MemTotal:       16384000 kB\nMemFree:        1000000 kB\n";
        assert_eq!(parse_mem_total_kib(sample), Some(16_384_000));
        assert_eq!(parse_mem_total_kib("garbage\n"), None);
    }

    #[test]
    fn default_memory_budget_clamps_ram_over_4() {
        let gib = 1024 * 1024 * 1024u64;
        assert_eq!(default_memory_budget_mib(gib), 512); // 256 MiB quarter -> clamped up
        assert_eq!(default_memory_budget_mib(2 * gib), 512); // exactly the floor
        assert_eq!(default_memory_budget_mib(8 * gib), 2048); // 2 GiB quarter, within range
        assert_eq!(default_memory_budget_mib(64 * gib), 4096); // 16 GiB quarter -> clamped down
    }

    #[test]
    fn extra_deserializes_default_settings_without_a_hash_map_alloc_issue() {
        // Sanity check that `Settings::default()` itself round-trips through `serde_json` (i.e.
        // the flatten + default combo is well-formed), independent of the file system.
        let json = serde_json::to_string(&Settings::default()).unwrap();
        let parsed: Settings = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, Settings::default());
    }
}

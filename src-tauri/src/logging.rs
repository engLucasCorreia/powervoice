//! File logging + crash logs (T-104).
//!
//! `tracing` logs go to a daily-rotating file in the OS state/log dir (Linux:
//! `~/.local/state/powervoice/logs/`, via `directories::ProjectDirs::state_dir()`), at a level
//! controlled by `POWERVOICE_LOG` (default `info`). A panic hook additionally writes a standalone
//! crash log (message + location + backtrace) to the sibling `crashes/` directory, so a crash is
//! diagnosable even if the last log lines were lost to the non-blocking writer's buffer.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use tracing_appender::non_blocking::WorkerGuard;

const DEFAULT_LOG_LEVEL: &str = "info";
const LOG_ENV_VAR: &str = "POWERVOICE_LOG";

fn project_dirs() -> Option<directories::ProjectDirs> {
    directories::ProjectDirs::from("app", "powervoice", "powervoice")
}

/// Where log files go: the OS state dir's `logs/` subfolder when the platform has one (Linux,
/// via `XDG_STATE_HOME`), else the data dir's (macOS/Windows: `directories` has no state dir
/// concept there — an unverified-platform fallback, MEMORY.md risk, not a bug). Pure function
/// over the two candidate base dirs so it's unit-testable without touching the real filesystem.
pub fn resolve_log_dir(state_dir: Option<PathBuf>, data_dir: PathBuf) -> PathBuf {
    state_dir.unwrap_or(data_dir).join("logs")
}

/// Same rule as [`resolve_log_dir`], for the crash-log directory.
pub fn resolve_crash_dir(state_dir: Option<PathBuf>, data_dir: PathBuf) -> PathBuf {
    state_dir.unwrap_or(data_dir).join("crashes")
}

fn log_dir() -> PathBuf {
    match project_dirs() {
        Some(dirs) => resolve_log_dir(
            dirs.state_dir().map(Path::to_path_buf),
            dirs.data_dir().to_path_buf(),
        ),
        None => std::env::temp_dir().join("powervoice").join("logs"),
    }
}

fn crash_dir() -> PathBuf {
    match project_dirs() {
        Some(dirs) => resolve_crash_dir(
            dirs.state_dir().map(Path::to_path_buf),
            dirs.data_dir().to_path_buf(),
        ),
        None => std::env::temp_dir().join("powervoice").join("crashes"),
    }
}

/// Resolves the `tracing_subscriber::EnvFilter` directive from `POWERVOICE_LOG`, defaulting to
/// `info`. Pure function (no direct env access) so default/override behavior is unit-testable
/// without mutating process-global env from parallel tests.
pub fn resolve_level_directive(env_value: Option<&str>) -> String {
    let trimmed = env_value.map(str::trim).filter(|v| !v.is_empty());
    trimmed.unwrap_or(DEFAULT_LOG_LEVEL).to_string()
}

/// Initializes file logging for the whole process and installs the panic hook. Returns a guard
/// that must be kept alive for the process's lifetime — dropping it stops the non-blocking
/// writer from flushing (see `lib.rs::run`, which binds it to a `let _guard = ...` that lives as
/// long as the Tauri event loop).
pub fn init() -> WorkerGuard {
    let dir = log_dir();
    let _ = std::fs::create_dir_all(&dir);
    let file_appender = tracing_appender::rolling::daily(&dir, "powervoice.log");
    let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);

    let directive = resolve_level_directive(std::env::var(LOG_ENV_VAR).ok().as_deref());
    let filter = tracing_subscriber::EnvFilter::try_new(&directive)
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(DEFAULT_LOG_LEVEL));

    // `set_global_default`/`.init()` can only succeed once per process; ignore a second call
    // (e.g. from a test harness that also calls `init()`) rather than panicking.
    let _ = tracing_subscriber::fmt()
        .with_writer(non_blocking)
        .with_ansi(false)
        .with_env_filter(filter)
        .try_init();

    install_panic_hook(crash_dir());
    guard
}

// --- Crash log -----------------------------------------------------------------------------------

fn crash_file_name(now: SystemTime) -> String {
    let millis = now
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    format!("crash-{millis}-{}.log", std::process::id())
}

/// Pure formatting of a crash report. Kept separate from the panic hook itself because
/// `std::panic::PanicHookInfo`'s fields aren't publicly constructible, so this is what a unit
/// test can actually call directly.
pub fn format_crash_report(
    message: &str,
    location: Option<&str>,
    backtrace: &std::backtrace::Backtrace,
) -> String {
    let mut report = String::new();
    report.push_str("PowerVoice crash report\n");
    report.push_str(&format!("message: {message}\n"));
    report.push_str(&format!("location: {}\n", location.unwrap_or("unknown")));
    report.push_str("backtrace:\n");
    report.push_str(&backtrace.to_string());
    report.push('\n');
    report
}

/// Writes a crash report to a timestamped file under `dir`, returning its path.
pub fn write_crash_log(dir: &Path, report: &str, now: SystemTime) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let path = dir.join(crash_file_name(now));
    let mut file = std::fs::File::create(&path)?;
    file.write_all(report.as_bytes())?;
    file.sync_all()?;
    Ok(path)
}

/// Installs a panic hook that writes a crash log (message + location + backtrace) to `dir`,
/// logs the panic via `tracing::error!`, then chains to whatever hook was previously installed
/// (so the default stderr dump — or an outer test's hook — still runs too).
pub fn install_panic_hook(dir: PathBuf) {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let message = if let Some(s) = info.payload().downcast_ref::<&str>() {
            (*s).to_string()
        } else if let Some(s) = info.payload().downcast_ref::<String>() {
            s.clone()
        } else {
            "<non-string panic payload>".to_string()
        };
        let location = info.location().map(ToString::to_string);
        let backtrace = std::backtrace::Backtrace::force_capture();
        let report = format_crash_report(&message, location.as_deref(), &backtrace);
        match write_crash_log(&dir, &report, SystemTime::now()) {
            Ok(path) => {
                tracing::error!(panic = %message, crash_log = %path.display(), "PowerVoice panicked");
            }
            Err(err) => {
                tracing::error!(panic = %message, write_error = %err, "PowerVoice panicked (failed to write crash log)");
            }
        }
        previous(info);
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(label: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "powervoice-logging-test-{label}-{}-{nanos}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn resolve_level_directive_defaults_and_overrides() {
        assert_eq!(resolve_level_directive(None), "info");
        assert_eq!(resolve_level_directive(Some("")), "info");
        assert_eq!(resolve_level_directive(Some("   ")), "info");
        assert_eq!(resolve_level_directive(Some("debug")), "debug");
        assert_eq!(resolve_level_directive(Some("  warn  ")), "warn");
    }

    #[test]
    fn resolve_log_dir_prefers_state_dir_over_data_dir() {
        let state = PathBuf::from("/state/powervoice");
        let data = PathBuf::from("/data/powervoice");
        assert_eq!(
            resolve_log_dir(Some(state.clone()), data.clone()),
            state.join("logs")
        );
        assert_eq!(resolve_log_dir(None, data.clone()), data.join("logs"));
    }

    #[test]
    fn resolve_crash_dir_prefers_state_dir_over_data_dir() {
        let state = PathBuf::from("/state/powervoice");
        let data = PathBuf::from("/data/powervoice");
        assert_eq!(
            resolve_crash_dir(Some(state.clone()), data.clone()),
            state.join("crashes")
        );
        assert_eq!(resolve_crash_dir(None, data.clone()), data.join("crashes"));
    }

    #[test]
    fn format_crash_report_includes_message_location_and_backtrace_marker() {
        let backtrace = std::backtrace::Backtrace::disabled();
        let report = format_crash_report("boom", Some("src/main.rs:1:1"), &backtrace);
        assert!(report.contains("boom"));
        assert!(report.contains("src/main.rs:1:1"));
        assert!(report.contains("backtrace:"));
    }

    #[test]
    fn format_crash_report_handles_missing_location() {
        let backtrace = std::backtrace::Backtrace::disabled();
        let report = format_crash_report("boom", None, &backtrace);
        assert!(report.contains("location: unknown"));
    }

    #[test]
    fn write_crash_log_creates_a_timestamped_file_with_the_report() {
        let dir = temp_dir("write");
        let now = SystemTime::UNIX_EPOCH + std::time::Duration::from_millis(1_700_000_000_000);
        let path = write_crash_log(&dir, "report contents", now).unwrap();
        assert!(
            path.file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .starts_with("crash-1700000000000-")
        );
        let contents = std::fs::read_to_string(&path).unwrap();
        assert_eq!(contents, "report contents");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// End-to-end: install the real panic hook, trigger a real panic through
    /// `catch_unwind`, and confirm a crash log lands on disk with the panic message in it.
    /// Installs and immediately restores the previous global hook to minimize the window where
    /// this test's hook could observe an unrelated panic from another test thread.
    #[test]
    fn panic_hook_writes_a_crash_log_end_to_end() {
        let dir = temp_dir("panic-hook");
        install_panic_hook(dir.clone());

        let result = std::panic::catch_unwind(|| {
            panic!("integration boom");
        });
        assert!(result.is_err());

        // Restore whatever hook preceded ours (the Rust default, absent other global state).
        let _ = std::panic::take_hook();

        let entries: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .collect();
        assert_eq!(entries.len(), 1, "expected exactly one crash log file");
        let contents = std::fs::read_to_string(entries[0].path()).unwrap();
        assert!(contents.contains("integration boom"));
        assert!(contents.contains("backtrace:"));

        std::fs::remove_dir_all(&dir).ok();
    }
}

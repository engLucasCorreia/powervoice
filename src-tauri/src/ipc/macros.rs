/// Declares every Tauri command exposed to the UI in one place (ADR-003).
///
/// `ipc_commands!(app_info, other_cmd, ...)` expands to:
/// - `invoke_handler::<R>()`: a small generic wrapper around `tauri::generate_handler!` (the
///   handler closure it returns is generic over the Tauri `Runtime`, so this can't be a plain
///   `const`/`static`);
/// - `COMMAND_NAMES: &[&str]`, the literal, snake_case command name list;
/// - `CommandName`, a unit enum (one variant per command) deriving `ts_rs::TS`, exported as a TS
///   string union whose members are exactly the command names;
/// - `COMMAND_NAME_VARIANTS`, one `CommandName` per command, used only by the unit test that
///   guards `COMMAND_NAMES` and `CommandName` against drifting apart.
///
/// Event names (`EventName`) follow the same pattern once a ticket needs events; T-004 has none.
#[macro_export]
macro_rules! ipc_commands {
    ($($name:ident),+ $(,)?) => {
        pub fn invoke_handler<R: tauri::Runtime>()
        -> impl Fn(tauri::ipc::Invoke<R>) -> bool + Send + Sync + 'static {
            tauri::generate_handler![$($name),+]
        }

        pub const COMMAND_NAMES: &[&str] = &[$(stringify!($name)),+];

        #[allow(non_camel_case_types)]
        #[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, ts_rs::TS)]
        #[ts(export, export_to = "bindings.ts")]
        pub enum CommandName {
            $($name),+
        }

        pub const COMMAND_NAME_VARIANTS: &[CommandName] = &[$(CommandName::$name),+];
    };
}

/// Declares every event name emitted to the UI in one place (ADR-003), mirroring
/// `ipc_commands!` above. `ipc_events!(notice, other_event, ...)` expands to:
/// - `EVENT_NAMES: &[&str]`, the literal, snake_case event name list;
/// - `EventName`, a unit enum (one variant per event) deriving `ts_rs::TS`, exported as a TS
///   string union whose members are exactly the event names, plus an `as_str()` for use with
///   `tauri::Emitter::emit`;
/// - `EVENT_NAME_VARIANTS`, one `EventName` per event, used only by the unit test that guards
///   `EVENT_NAMES` and `EventName` against drifting apart (same pattern as `COMMAND_NAME_VARIANTS`).
#[macro_export]
macro_rules! ipc_events {
    ($($name:ident),+ $(,)?) => {
        pub const EVENT_NAMES: &[&str] = &[$(stringify!($name)),+];

        #[allow(non_camel_case_types)]
        #[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, ts_rs::TS)]
        #[ts(export, export_to = "bindings.ts")]
        pub enum EventName {
            $($name),+
        }

        impl EventName {
            pub fn as_str(self) -> &'static str {
                match self {
                    $(EventName::$name => stringify!($name)),+
                }
            }
        }

        pub const EVENT_NAME_VARIANTS: &[EventName] = &[$(EventName::$name),+];
    };
}

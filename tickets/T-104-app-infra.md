# T-104 — App infrastructure: logging, errors & notices, settings, stores, keymap, WebKit default

- **Milestone / wave:** M1 / W1
- **Tier:** Sonnet
- **Depends on:** M0 (T-009 rename merged)
- **Spec refs:** SPEC-001 §2.3 (notices/banners), §2.5 (persisted device prefs), SPEC-002 §3 (persisted monitor mode, default format), SPEC-003 §2.5 + §3 (shortcuts, telemetry rate), SPEC-004 §2.4 (memory budget setting) · **ADR refs:** ADR-003 (IpcError, events), ADR-009 Amendment 1 · MEMORY D-014, D-016

## Goal
The cross-cutting app plumbing every M1 feature uses: file logging, crash logs, typed errors shown as
toasts/banners, a versioned settings file, Svelte state stores with typed command wrappers, a central
keymap registry, and the Linux WebKit DMA-BUF default.

## Scope
**In:**
- `tracing` logging to a rotating file in the OS state/log dir (Linux `~/.local/state/powervoice/logs/`), level via `POWERVOICE_LOG`; panic hook writing a crash log with backtrace.
- `IpcError { code, key, params }` completed: error codes enum, i18n keys, `From` conversions in `src-tauri`; UI toast + persistent banner components (`notice` event per ADR-003) — all strings via i18n.
- Settings service (in `src-tauri`, schema types shared via ts-rs): versioned JSON in the OS config dir, atomic writes, migration hook, defaults: device prefs (SPEC-001 §2.5; engine type from T-102 is re-declared as DTO if T-102 isn't merged yet — coordinate via a small `settings` module), `default_format` 48 kHz / 24-bit, `monitor_mode` Off, `telemetry_rate_hz` 60, `memory_budget` clamp(RAM/4, 512 MiB, 4 GiB).
- Svelte 5 runes stores pattern (`ui/src/lib/state/`) + typed command wrappers; settings store with load/save.
- **Keymap registry** (`ui/src/lib/keymap/`): single source of bindings and action ids, platform-aware (Ctrl ↔ ⌘). Defaults: Space = play/pause, **Shift+Space = play from start**, Home = return to start, **Shift+R = record (provisional)**, M = add marker, Ctrl+Z = undo, Ctrl+Shift+Z = redo. Actions are dispatched to handlers registered by features later; unknown/unhandled actions are no-ops. No remapping UI (v1).
- **WebKit DMA-BUF default** (ADR-009 Amendment 1): on Linux, `main.rs` sets `WEBKIT_DISABLE_DMABUF_RENDERER=1` before the WebView is created unless it is already set or `POWERVOICE_WEBKIT_DMABUF=1`; `just dev`/`just spike` follow the same rule; remove the old `POWERVOICE_WEBKIT_SAFE` hook and document the opt-out in `ui/README.md`.

**Out:** device/transport/recording features themselves, settings UI panels (T-109).

## Crates / files
`src-tauri/src/{logging,settings,ipc/error}.rs`, `src-tauri/src/main.rs`, `ui/src/lib/{state,keymap,notices}/`, `ui/src/lib/i18n/en.json`, `justfile`. Allowed new deps: `tracing`, `tracing-subscriber`, `tracing-appender`, `directories` (in `src-tauri` only).

## Acceptance tests to write
- [ ] Settings: defaults on first run; round-trip; unknown future fields preserved; v0→v1 migration test; atomic write (temp + rename).
- [ ] Panic hook writes a crash log file (test with a forced panic in a child process or unit-level hook test).
- [ ] IpcError → toast renders the i18n text with params (Vitest + mockIPC).
- [ ] Keymap: each default binding resolves to its action; Shift+Space ≠ Space; Ctrl ↔ ⌘ mapping on macOS; bindings are unique.
- [ ] WebKit env rule: unit-test the decision function (unset → set; already set → untouched; opt-out → untouched; non-Linux → untouched).
- [ ] `just check` green.

## Definition of Done
- [ ] Tests first, passing; every user-facing string via i18n; report in CLAUDE.md format.

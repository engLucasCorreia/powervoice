# S3-01 — Rack panel + generic parameter UI (Slice 3)

- **Tier:** Sonnet
- **Depends on:** S1-01 (engine owns `RackHost`/`LiveRack`), H-01 (rack warm-up/bypass fix — can run in parallel; merge H-01 first if both are ready)
- **Specs (essential subset only):** SPEC-012 §2.1 rack panel (slots in processing order, add from the registry menu, remove, drag to reorder, per-slot bypass, whole-rack A/B listening-only, latency readout), §2.4 parameter changes (≤ 1 per animation frame while dragging, latest wins), §2.6 generic parameter UI derived only from the schema (widgets by flags/taper, double-click reset, text entry parsed by Rust, displayed text always from Rust `param_changed`).
- **Read first:** CLAUDE.md, MEMORY.md (D-022; T-103 learnings: RackHost API, `ParamChanged` coalesced per tick), crates/rack, crates/module-api (ParamInfo, flags, tapers), the S1-01 `Engine` API.

## Goal
The owner builds a voice chain in the right-hand rack panel and tweaks every parameter of every module with a working, click-free UI — for any module that exists now or later.

## Scope (in)
- `src-tauri` commands/events: `rack_list_modules`, `rack_add(module_id, index)`, `rack_remove(slot)`, `rack_move(from, to)`, `rack_bypass(slot, on)`, `rack_ab(on)`, `param_set_normalized(slot, id, v)`, `param_set_text(slot, id, text)`, `rack_changed`, `param_changed` (text + normalized), `rack_latency`; DTOs via ts-rs for the parameter schema.
- UI: rack panel (right dock) with slot headers (power, name, latency, menu Remove), drag-reorder, Add Module menu grouped by features, A/B toggle with "listening only" badge; generic parameter panel (slider + value field, toggles, dropdowns, detented sliders, read-only readouts, Shift = fine, wheel, double-click default, text entry with error outline).
- Rack state persisted in the session (existing rack model) so it survives reopen within the app session; sidecar persistence is hardening (T-306).
- Tests: Vitest with a 40-parameter schema fixture (SPEC-012 AC-12 subset), mockIPC command flows; Rust: rack commands round-trip through the engine on the fake backend.

## Handoff from H-01 (merged)
- After `SlotRestarted` the host sends `LatencyChanged` but no `ParamChanged` for the (re)created parameters → the UI must re-read `slot_info` on `SlotRestarted`.
- `replace_state()` on a failed-at-load slot still returns `NotLoaded` (only `restart()` retries).

## Handoff from S3-02 (merged)
- Hide a parameter group whose enable parameter and all body parameters are `HIDDEN` (Dynamics AutoGate/Expander stubs), otherwise empty section headers appear.
- Module i18n keys `module.dynamics.*`, `module.noise_gate.*`, `param.gain.*` are not in `en.json` yet — add them (names, groups, telemetry, enum labels).

## Out (deferred to hardening)
Presets (T-406), custom module panels (EQ graph, dynamics graph — come with their module tickets), filter box for > 32 params, placeholders UI polish, latency compensation of the playhead (T-401).

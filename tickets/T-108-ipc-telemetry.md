# T-108 — IPC layer: commands/events for M1 features, telemetry & recording channels, binary decoders, clock sync

- **Milestone / wave:** M1 / W3
- **Tier:** Sonnet
- **Depends on:** T-104, T-105 (T-106/T-107 command surfaces can be stubbed and filled when they merge)
- **Spec refs:** SPEC-000 AC-7, SPEC-003 §2.2 + AC-6 (UI extrapolation), SPEC-001/002 command surfaces · **ADR refs:** ADR-003 (all), ADR-009 §1

## Goal
The UI can drive every M1 engine feature through typed commands and receive state, notices and 60 Hz
binary telemetry, with a verified Rust↔TS binary contract.

## Scope
**In:**
- Commands (via `ipc_commands!`, async, ts-rs DTOs): devices (list, select host/input/channel/output, rate, buffer, rescan), settings get/set, transport (play, pause, stop, play_from_start, return_to_start, seek, loop), arm/record/stop_record/new_recording, monitor mode, rack (M1 minimal: list modules, add/remove/move slot, bypass, A/B, set_param_normalized/text for Gain), `clock_now_ns`.
- Events: `devices_changed`, `transport_state`, `document_changed`, `rack_changed`, `param_changed`, `notice`.
- Channels: `telemetry_subscribe` (`VXTM`, 60 Hz), `recording_subscribe` (`VXRP`).
- `ui/src/lib/ipc/binary.ts` decoders (DataView, zero-copy) + golden fixtures written by a Rust `gen_ipc_fixtures` test into `ui/src/lib/ipc/__fixtures__/` and decoded in Vitest (SPEC-000 AC-7).
- UI clock sync (5 samples, min RTT, every 30 s) and playhead extrapolation module with slew/jump rules (ADR-003 §3).

**Out:** visual components (T-109), peaks/spectrogram (M2).

## Crates / files
`src-tauri/src/ipc/*`, `ui/src/lib/ipc/*`, `ui/src/lib/state/*`.

## Acceptance tests to write
- [ ] SPEC-000 AC-7 golden-frame contract (VXTM, VXRP now; VXPK/VXST fixtures when M2 adds them).
- [ ] Command-name drift guard covers every new command; events likewise.
- [ ] SPEC-003 AC-6 extrapolation unit test (Vitest, synthetic anchors + fake clock, ±10 ms).
- [ ] mockIPC tests for each store's command wrappers.

## Definition of Done
- [ ] Tests first, passing; `just check` green (incl. stale-bindings check); report in CLAUDE.md format.

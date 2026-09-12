# T-102 — Devices: backend trait, cpal + fake backends, enumeration, channel pick, hot-plug, device loss

- **Milestone / wave:** M1 / W1
- **Tier:** Opus (+ Opus review)
- **Depends on:** M0 (T-009 rename merged)
- **Spec refs:** SPEC-001 (all ACs, engine side), SPEC-002 AC-16 (output-only loss detection) · **ADR refs:** ADR-002 §1, §7, §8, ADR-001 §3 · **PROMPT refs:** §2, §3.1

## Goal
`vox-engine` has a device layer that the engine core (T-105) builds on: one `Backend` trait with a real
cpal implementation and a deterministic fake, device enumeration and selection with fallback rules,
input-channel deinterleaving, hot-plug polling and a device-lost/recovery state machine.

## Scope
**In:**
- `Backend` trait: hosts, devices (input/output, `(host, name)` keys), capabilities (rates, buffer range, channels), `open_input` / `open_output` with RT callbacks and per-stream timestamps, error flags (`AtomicU32`: `DEVICE_LOST`, `BACKEND_ERROR`).
- cpal backend behind default feature `backend-cpal`; cpal 0.18 with `pipewire` and `jack` features on Linux; **default host PipeWire when available, else ALSA** (SPEC-001 §2.1).
- Fake backend: scripted device sets, hot-plug/unplug events, loss injection per stream, random callback sizes (1…4096), synthetic capture/playback timestamps, clock skew in ppm, nominal rate mismatch.
- Device-poll thread (~1 s) producing add/remove diffs; manual rescan.
- Input channel selection by deinterleaving (changing channel needs no stream reopen).
- Pure functions: sample-rate fallback, buffer-size clamping (SPEC-001 §2.2).
- Pure device-lost / recovery state machine incl. the owner exception: output-only loss while recording does **not** stop recording (SPEC-001 §2.4).
- `DevicePrefs` model (serde) with "saved intent vs applied value" semantics (SPEC-001 §2.5); file persistence is T-104's settings service.

**Out:** transport, reader, audio processing in callbacks (T-105), recording (T-106), UI (T-109).

## Crates / files
`crates/engine` (`backend/{mod,cpal,fake}.rs`, `devices.rs`, `device_state.rs`, `prefs.rs`). Allowed new deps: `cpal` 0.18 (optional, feature `backend-cpal`), `rtrb` 0.4, `serde`.

## Acceptance tests to write
- [ ] SPEC-001 AC-1…AC-8, each via the fake backend (and unit tests for the pure functions), per the SPEC-001 test plan.
- [ ] Deinterleave: 2-channel fake input with tone on ch1 / silence on ch2 and vice versa → selected channel only.
- [ ] Output-only `DEVICE_LOST` while the state machine is in "recording" keeps recording (SPEC-002 AC-16 state-machine part).
- [ ] The crate builds and all tests pass with `--no-default-features` (no cpal) — SPEC-000 AC-5.
- [ ] Manual smoke note in the report: enumerate real devices on the owner's PipeWire machine via a small `examples/list_devices.rs`.

## Definition of Done
- [ ] Tests first, passing; `just check` and `just check-cross` green; RT rules respected in callback wrappers; report in CLAUDE.md format.

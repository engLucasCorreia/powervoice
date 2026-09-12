# T-103 — Rack core: registry, live chain swap, host bypass & A/B, coalescing, offline render, Gain, CLI render

- **Milestone / wave:** M1 / W1
- **Tier:** Opus (+ Opus review)
- **Depends on:** M0 (T-009 rename merged)
- **Spec refs:** SPEC-012 (M1 ACs: AC-1…AC-11, AC-13…AC-15), SPEC-000 AC-3 (CLI side) · **ADR refs:** ADR-005, ADR-002 §2–§3 (return ring), ADR-001 §5 · **PROMPT refs:** §3.4, §3.7 · MEMORY "T-103 handoff" rules

## Goal
The rack behaves like SPEC-012 says in M1: live, click-free slot edits that keep untouched modules
warm; host-owned bypass and whole-rack A/B; no lost parameter values; offline render equal to
realtime; the built-in Gain module; `powervoice-cli render --rack`.

## Scope
**In:**
- Module **registry** (id → factory, one version per id) in `vox-rack`; composition roots register built-ins.
- `RackModel` (serde, sidecar slot schema per ADR-005 §10) and chain instantiation from it; **placeholders** for unknown ids (dry, latency 0, JSON round-trips verbatim).
- Live chain **swap/retire**: new chain built off the audio thread, swapped in with a 15 ms crossfade; untouched instances moved (or any mechanism meeting SPEC-012 AC-1); retired objects leave through a return ring (SPSC `rtrb`) — never dropped on the audio thread.
- Host bypass per slot (latency-matched dry, 15 ms linear equal-gain crossfade) and whole-rack A/B (dry delayed by total latency; listening-only — offline renders ignore it).
- Dual-mono shim for stereo-only modules.
- Event routing with **same-offset/same-id coalescing in place** (SPEC-012 §4.2); overflow carry-over (already in T-005) kept.
- **RT drain** of module output events (READ_ONLY reports) to the control side.
- **Non-finite guard** (SPEC-012 §2.9); offline render aborts with a slot-naming error.
- `rack::offline::render` (4096-frame blocks, offline mode, latency trim, same length, time-aligned).
- Built-in **Gain** `org.powervoice.gain@1.0.0` in `crates/modules` (SPEC-012 §3: −60 = −∞ … +24 dB, 20 ms linear ramp).
- §4.3 zipper/click measurement harness in `module-api` test-util + calibration modules `TestHardStep`, `TestStair64`; test modules `TestDelay(n)`, `TestNaN`, `TestReporter`, `TestRestart`.
- `powervoice-cli render --rack <rack.json> <in.wav> <out.wav>` (unknown id → error listing ids; 32f output).

**Out:** latency *compensation* of the playhead and `latency_changed` re-activation flow (T-401), rack UI/presets (T-405/T-406), other modules (M4/M5).

## Crates / files
`crates/rack`, `crates/modules`, `crates/module-api` (test-util harness), `crates/cli`. Allowed new deps: `rtrb` 0.4, `serde`/`serde_json`.

## Acceptance tests to write
- [ ] SPEC-012 AC-1…AC-11, AC-13…AC-15 as specified (realtime paths driven by a minimal block-feeding test driver with random block sizes; the fake-backend end-to-end versions are added in T-105).
- [ ] Harness calibration: TestGain passes §4.3; TestHardStep and TestStair64 fail.
- [ ] Gain passes `ModuleTestHost` with no `allow_delayed_effect` opt-outs.
- [ ] SPEC-000 AC-3 (CLI render bit-identical to `rack::offline::render`).

## Definition of Done
- [ ] Tests first, passing; `just check` green; no allocation in `process` paths (via `test_util::no_alloc`); report in CLAUDE.md format.

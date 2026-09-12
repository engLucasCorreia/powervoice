# T-005 — `module-api` v0 + `rack` skeleton + module test host

- **Milestone / wave:** M0 / W2
- **Tier:** Opus
- **Depends on:** T-001, T-003 (ADR-005)
- **PROMPT refs:** §2, §3.4, §3.7, §4 · **ADR refs:** ADR-005, ADR-002

## Goal
The Module API from ADR-005 exists as code with a test host that can hammer any module, and the `rack` crate can run a chain of modules (without bypass crossfades or latency compensation yet — those are T-103/T-401).

## Scope
**In:**
- `crates/module-api`: every type and trait in ADR-005 (descriptor, params + flags + taper + text conversion, per-block event list with sample offsets, `ProcessContext`, `ProcessMode`, `Tail`, `ModuleState` + migrate hook, `trait Module`, extension traits `Telemetry`/`ResponseCurve`/`NoiseProfile` and the query mechanism). No dependencies except `serde` (+ derive) for state/descriptor serialization, if ADR-005 calls for it.
- Feature `test-util` in `module-api` providing `ModuleTestHost`: activates a module, feeds signals in **random block sizes (0..=max_block)**, injects random param events at random offsets, runs `process` under `assert_no_alloc`, asserts outputs are finite, checks `reset()` is RT-safe, and round-trips state save/load.
- `crates/rack`: `Rack` with an ordered `Vec` of slots, `activate`/`process` through the chain with per-slot event lists split at block boundaries, total latency = sum of slot latencies (reporting only). Reorder/insert/remove API (off-audio-thread construction; the swap mechanism can be a simple placeholder documented for T-103).
- A reference module in `module-api` tests (e.g. `TestGain` with a smoothed gain parameter) proving the API is usable.

**Out:** host bypass/crossfade, latency compensation, cpal, real DSP modules, UI.

## Acceptance tests
- [ ] `TestGain` passes `ModuleTestHost` (random blocks incl. 0-length, random events, no alloc, finite output).
- [ ] Param event at offset k in a block takes effect from sample k (within the module's documented smoothing).
- [ ] State round-trip: save → load into a fresh instance → identical output on the same input (bit-exact).
- [ ] `Rack` with 3× `TestGain` (−6 dB each) yields −18.00 ± 0.01 dB on a 1 kHz sine.
- [ ] `ParamInfo` text conversion round-trips for linear, log and dB tapers.
- [ ] `just check` green.

## Definition of Done
- [ ] Above green; public API has doc comments; report lists any deviation from ADR-005.

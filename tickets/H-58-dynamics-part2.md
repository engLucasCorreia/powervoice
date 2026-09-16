# H-58 — Dynamics part 2: activate the Expander and AutoGate (T-407 remainder)

- **Tier:** Opus (DSP plus RT; blocking findings only)
- **From:** T-407's row — "S3-02 (params implemented); rest: verify SPEC-016 part-2 ACs". H-51 documented the current state: AutoGate, Expander and look-ahead are schema-present but `HIDDEN`/inert (`NOT_YET_AVAILABLE`), `latency_samples()` is always 0, and their telemetry channels always read 0.
- **Read first:** CLAUDE.md (RT rules: no allocation, locks or unbounded loops in `process()`), MEMORY.md (S3-02 dynamics, T-103 rack latency and restart, T-401 latency compensation, H-46 pre-roll, T-110 benches and the per-module CPU budgets in `spec_budgets`, H-51's implementation-status note), specs/SPEC-016 (the whole spec, especially part 2: AutoGate, Expander, look-ahead, their ACs and telemetry), specs/SPEC-012 (latency reporting and restart), `crates/modules/src/dynamics.rs`, `crates/dsp`.

## Scope (in)
1. Implement and **un-hide** AutoGate and the Expander to SPEC-016 part 2, with their parameters, ranges, defaults, units and telemetry channels (gain reduction per section).
2. **Look-ahead**: report the real `latency_samples()`, trigger the rack's restart/compensation path when it changes (T-103/T-401), and keep `process()` allocation-free.
3. Update the module's CPU budget bench row (`spec_budgets`) and keep it inside the spec's budget.
4. Remove the `NOT_YET_AVAILABLE` marks and H-51's "implementation status" note for whatever you make live; leave the note accurate for anything still deferred.

## Tests
- Every SPEC-016 part-2 acceptance criterion.
- Golden-signal tests for gate and expander behaviour (thresholds, attack, release, hold, ratio) at 44.1 and 48 kHz.
- Latency reported and compensated; a latency change restarts cleanly with no click.
- `no_alloc` on `process()`.

`just check` and `just bench` (the module budget row) must pass.

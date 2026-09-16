# H-59 — Engine, recording, IPC and UI hardening audit (T-105/T-106/T-108/T-109 remainder)

- **Tier:** Opus (audit first, then fill gaps; blocking findings only)
- **From:** four rows whose vertical slices shipped a subset: engine core, recording, the IPC layer and the M1 UI, each marked "rest hardening".
- **Method:** this is an audit like T-206, T-401 and T-810. Produce a matrix of every acceptance criterion in the specs below — implemented / partial / missing — put it in your report and in the ticket's own section of `docs/performance.md`-style prose if it helps, then fill only the real gaps. Don't rewrite what already works.
- **Read first:** CLAUDE.md, MEMORY.md (all T-1xx, S1-xx, H-xx entries for these areas), specs/SPEC-001 (engine/devices), SPEC-002 (recording/monitoring), SPEC-003 (transport/playback), SPEC-005 (files/multichannel), SPEC-018 (sessions/sidecar), SPEC-022 (punch-in), docs/adr/ADR-002 (threading/RT) and ADR-003 (IPC paths), `crates/engine`, `crates/project`, `src-tauri`, the device/transport/meter UI.

## Scope (in)
1. The audit matrix for SPEC-001, SPEC-002, SPEC-003 and ADR-003's IPC contract (commands, events, binary channels, clock sync, golden fixtures).
2. Fill the gaps that matter to a user: device loss and recovery paths, monitoring modes, error and notice coverage, sample-rate and channel edge cases, clock-sync drift, and any binary decoder without a golden fixture.
3. Anything you judge out of scope (large features rather than hardening) goes in your report as a proposed ticket, not silently skipped.

## Tests
One per gap filled, plus golden fixtures for any binary frame that lacks one.

`just check` must pass.

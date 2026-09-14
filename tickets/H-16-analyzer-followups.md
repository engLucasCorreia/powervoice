# H-16 — Analyzer follow-ups (T-208) + telemetry rate setting

- **Tier:** Sonnet (no review loop; blocking findings only)
- **Depends on:** T-208 (live analyzer), T-306 (sidecar view state), H-12 (editor view) if merged.
- **Read first:** CLAUDE.md, MEMORY.md (T-208, H-03, T-306 notes), specs/SPEC-007 §2.9 (analyzer UI), ADR-003 Amendments 1/3/4, ui/src/lib/analyzer/*, ui/src/lib/state/settings.svelte.ts, src-tauri/src/settings.rs, crates/engine/src/{telemetry.rs,analyzer.rs,control.rs}.

## Scope (in)
1. **Analyzer UI completeness (SPEC-007 §2.9):** display floor/ceiling, frequency-range zoom/pan on the log axis, hover readout (frequency, band level), "No output device" state from the real device status, View → Analyzer menu toggle.
2. **Persistence:** analyzer visibility, response (Fast/Medium/Slow) and peak-hold in app `Settings`.
3. **Telemetry rate:** `Settings.telemetry_rate_hz` (60 / 30 Hz) is stored but wired to nothing — make the control-thread publishers (VXTM, VXMT, VXSA) honour it (frame every tick at 60 Hz, every other tick at 30 Hz, or a rate-driven accumulator), changeable at runtime.

## Tests
Settings round-trip; 30 Hz setting halves the frame count over a simulated second for each publisher; UI floor/ceiling/zoom math (canvas-free modules).

## Out
Analyzer overlay on the EQ graph (needs SPEC-015 reconciliation).

`just check` must pass.

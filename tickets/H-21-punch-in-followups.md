# H-21 — Punch-in / record-at-cursor follow-ups (T-304)

- **Tier:** Sonnet (Opus if the engine path needs changes; blocking findings only)
- **Depends on:** T-304 (record operations, calibration), H-19 (menu bar), T-301 (recovery).
- **Read first:** CLAUDE.md (RT rules), MEMORY.md (T-304 notes: operation flow, `op_try_seal` on every end path, fake `Loopback`, virtual heard positions; H-05 crash-test pattern), specs/SPEC-022 (ACs 3, 5–7, 10, 11, 13–16, 18, 19, 21), SPEC-002, crates/engine/src/{record_op.rs,control.rs,capture.rs,reader*.rs}, crates/project/src/{take.rs,edit.rs}, src-tauri/src/recording.rs, ui/src/lib/record/*, ui/src/lib/waveform/*.

## Scope (in), priority order
1. **Markers during an operation (AC-10):** `marker_add` works while a record operation runs (markers land at the heard virtual position and follow the committed window; a cancelled op leaves them as a separate "Add Marker" entry).
2. **Waveform during an operation:** draw the live take at `at` (punch region overlay; Insert shifts the displayed waveform after `at`).
3. **Device loss per phase (AC-14)** and **Record while playing** (stop first): tests for each phase.
4. **Talent-source exactness:** a perfectly timed "talent" source in the fake backend so AC-3/AC-5 (talent X), AC-6 at ±200 ppm drift, AC-7's pooled noise check and AC-13 (dropouts during a punch) run at the engine level.
5. **AC-11** seeded 20-operation sequence (hash + undo/redo), **AC-15** with a killed child process.
6. **Settings → Recording page** (punch pre/post-roll defaults, mode, offsets per device) inside a Preferences dialog shared with H-17's "Memory for audio".

## Tests
One per item, SPEC-022 AC names in test names. RT callbacks stay no-alloc.

## Out
AC-20 (owner manual smoke), AC-21 cost counters (T-704 performance).

`just check` must pass.

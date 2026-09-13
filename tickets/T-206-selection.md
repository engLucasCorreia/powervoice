# T-206 — Selection model, time formats, zero-crossing snap

- **Milestone / wave:** M2 / W3
- **Tier:** Sonnet
- **Depends on:** T-205
- **Spec refs:** SPEC-006 (selection: click-drag, Shift+click extend, double-click / Ctrl+A select all, Esc clears, handles 6 px, exact sample boundaries at every zoom; time display formats; zero-crossing snap ±512 samples on selection edges), SPEC-003 (Play from start uses the selection start; loop uses the selection) · **ADR refs:** ADR-003 · MEMORY A-001

## Goal
A time selection that is sample-exact at every zoom, shared by transport (loop, play from start) and,
from M3, by editing.

## Scope
**In:** selection state in the document store (document samples, `u64` as number), mouse interactions on the waveform and rulers, keyboard (Ctrl+A, Esc), selection handles, selection readout (start/end/duration in the chosen time format), time-format utilities (timecode/samples/seconds parse + format, exact round trips), zero-crossing snap setting and search (Rust command over the snapshot, ±512 samples, silent no-op when none), engine sync of the selection for loop/play-from-start commands.

**Out:** edit ops (M3), marker editing (M3).

## Acceptance tests to write
- [ ] SPEC-006 selection/time-format/snap ACs per its test plan (property tests for pixel ↔ sample mapping; exact boundaries at extreme zoom levels; snap lands on a sign change).

## Definition of Done
- [ ] `just check` green; report in CLAUDE.md format.

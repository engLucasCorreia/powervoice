# H-23 — Punch-in polish (H-21 open questions, A-016)

- **Tier:** Sonnet (no review loop; blocking findings only)
- **Depends on:** H-21.
- **Read first:** CLAUDE.md (RT rules), MEMORY.md (T-304, H-21 notes; A-016), specs/SPEC-022, crates/engine/src/{control.rs,record_op.rs,backend/fake.rs}, ui/src/lib/waveform/{WaveformView.svelte,opLayout.ts}, ui/src/lib/state/waveformView.svelte.ts.

## Scope (in)
1. **No phantom PostRoll (A-016):** when the output device is lost mid-window, don't emit a `PostRoll` phase event when the heard position passes `E` (no post-roll plays); go straight to the commit/stop path the spec defines for device loss.
2. **Follow the record head (A-016):** during a record operation the waveform scrolls to keep the record head in view (page-flip like the playhead follow during playback, respecting a user scroll until the next page).
3. **AC-13 "length unknown" variant:** add an unreliable-timestamp flag to the fake backend and test SPEC-022 AC-13's length-unknown dropout case.

## Tests
One per item.

`just check` must pass.

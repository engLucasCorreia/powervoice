# T-303 — Markers: kinds, add/region, rename, drag, delete, navigation, Markers panel

- **Milestone / wave:** M3 / W2
- **Tier:** Sonnet
- **Depends on:** T-301 (history labels), M2 (T-205 waveform, T-206 selection, T-108 clock sync)
- **Spec refs:** SPEC-009 (all ACs), SPEC-004 (marker-only edits don't stop playback, undo semantics), SPEC-008 §4.2 (marker shifting), SPEC-005 (cue/adtl), SPEC-002 (dropout markers, markers while recording) · **ADR refs:** ADR-004 Amendment 2 (marker kind) · MEMORY A-005

## Goal
Markers work exactly as SPEC-009 specifies — added precisely at the heard position, edited directly on
the waveform or in the panel, navigable by keyboard, undoable without interrupting playback.

## Scope
**In:**
- `vox-project`: `MarkerKind` (`User`, `Dropout`, `Unknown(String)`), ordering by `(pos, id)`, marker ops as undoable marker-only edits (add point/region, rename, move, resize, delete selected/all/filtered), name normalization, default names "Marker NN", limits (10 000 user / 100 000 total).
- Engine/IPC: marker commands (revision-guarded), heard-position placement from the UI's `KeyboardEvent.timeStamp` through the clock-sync mapping, markers allowed during recording (routed into the take).
- UI: marker flags and region spans on the waveform (drag after 3 px, 6 px snapping to cursor/selection/markers, Alt disables, Shift-drag region, Esc cancels); Markers panel (left dock 300 px, virtualized list, Name/Start/End/Duration/Type columns, numeric-aware sort, filter, inline rename, typed Start/End/Duration, multi-select sets the time selection to the span); navigation next/previous with the SPEC-009 rules; notices with Undo; keymap: M, `/`, F2, Ctrl+0, Ctrl+Alt+0, Ctrl+Alt+→/← (digits matched by physical key); Delete follows focus.

**Out:** sidecar persistence and open precedence (T-306), cue writing (T-201, already done).

## Acceptance tests to write
- [ ] SPEC-009 AC-1…AC-20 per its test plan (project unit tests for ops/ordering/limits; Vitest with mockIPC for panel/keys/drag; fake-backend placement accuracy test).

## Definition of Done
- [ ] Tests first, passing; `just check` green; report in CLAUDE.md format.

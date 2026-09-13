# S2-03 — Markers basic (Slice 2)

- **Tier:** Sonnet
- **Depends on:** S1-03 (waveform), S2-01 (edit plumbing)
- **Specs (essential subset only):** SPEC-009 — M adds a point marker at the playhead/cursor (heard position while playing), region from the selection when stopped; Markers panel (name, start, duration) with click-to-jump and inline rename; delete selected (Ctrl+0); next/previous (Ctrl+Alt+→/←); markers drawn on the waveform; marker ops are undoable and don't stop playback (SPEC-004); WAV `cue`/`LIST adtl` written on save and read on open (SPEC-005, via S1-02's `vox-io`).
- **Read first:** CLAUDE.md, MEMORY.md (D-022, A-005 markers), `vox-project` marker ops, S1-02/S1-03 APIs.

## Goal
The owner can drop markers while listening, see and rename them in a panel, jump between them, and they survive Save/Open in the WAV file.

## Scope (in)
- Engine marker commands (add point/region, rename, delete, move-by-panel-edit) → `Session` marker edits; heard-position placement from the UI key timestamp via clock sync.
- `vox-io`: RIFF `cue `/`LIST adtl` writer + reader (UTF-8 names, regions via `ltxt`); save/open integration.
- UI: marker flags on the waveform, Markers panel (left dock), keys M / Ctrl+0 / Ctrl+Alt+→/←.
- Tests: marker at heard position within ±10 ms (fake backend + extrapolation), cue round-trip exact in samples and names, undo of marker ops without stopping playback.

## Out (deferred to hardening)
Drag on the waveform, snapping, filter/sort, marker kinds UI, limits, sidecar precedence rules, delete all/filtered.

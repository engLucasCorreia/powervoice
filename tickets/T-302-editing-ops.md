# T-302 — Editing operations: cut, copy, paste, delete, trim, silence, insert silence, clipboard

- **Milestone / wave:** M3 / W2
- **Tier:** Sonnet (+ Opus review)
- **Depends on:** T-301
- **Spec refs:** SPEC-008 (all ACs), SPEC-004 (AC-6 stop-before-commit, AC-7 marker shifting), SPEC-006 (selection) · **ADR refs:** ADR-004 (piece splices, marker shifting, destructive-edit sequence), ADR-003 Amendment 1 (`clipboard_changed`) · MEMORY A-002 and the T-302 follow-ups

## Goal
The seven editing operations behave exactly as SPEC-008 specifies — sample-exact pure splices, correct
markers, instant on long documents, undoable with proper labels.

## Scope
**In:** `vox-project` piece-table ops (cut/copy/paste-at-cursor/paste-over-selection/delete/trim/silence/insert silence) with marker rules (SPEC-008 §4.2), adjacent-piece merging, `Piece.len` u32 splitting for long silence; in-app clipboard (pieces; materialized before session GC; re-bound after first same-rate paste; rate-converted paste via `dsp::resample` with a notice; converted pieces cached per document); revision-guarded engine commands + `clipboard_changed` event; Edit menu + waveform context menu; keymap bindings Ctrl+X, Ctrl+C, Ctrl+V, Delete, Ctrl+T; Insert Silence dialog (seconds / timecode / `N smp`); undo labels (i18n keys).

**Out:** mix paste, copy to new, zero-crossing adjust commands (out of scope per SPEC-008).

## Acceptance tests to write
- [ ] SPEC-008 AC-1…AC-19 per its test plan (property tests on random piece tables vs a flat-buffer reference model; exact marker positions; timing bench).

## Definition of Done
- [ ] Tests first, passing; `just check` green; Opus review passed; report in CLAUDE.md format.

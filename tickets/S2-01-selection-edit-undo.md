# S2-01 — Selection, cut/copy/paste/delete/trim/silence, undo/redo (Slice 2)

- **Tier:** Sonnet (Opus review only if the engine's stop → commit → swap sequence changes)
- **Depends on:** S1-01 (engine), S1-03 (waveform view)
- **Specs (essential subset only):** SPEC-006 selection (click-drag, Shift+click extend, Ctrl+A / double-click select all, Esc clears, exact sample boundaries); SPEC-008 ops (cut, copy, paste at cursor / over selection, delete, trim to selection, silence) with its marker mapping (already in `vox-project`), post-op selection/cursor table, in-app clipboard (pieces); SPEC-004 §2.3 destructive edit stops playback before commit, undo/redo exactness; keys Ctrl+X/C/V, Delete, Ctrl+T, Ctrl+Z, Ctrl+Shift+Z.
- **Read first:** CLAUDE.md, MEMORY.md (D-022; T-101 learnings — edits only through `Session`), `crates/project` (History, Edit ops, MarkerMapping), the S1-01 `Engine` API.

## Goal
The owner can select audio on the waveform, cut/copy/paste/delete/trim/silence it, and undo/redo — sample-exact and instant.

## Scope (in)
- Selection model in the UI store (document samples), drawn on the waveform, synced to the engine (for Play from start / edits).
- Engine commands `edit_cut|copy|paste|delete|trim|silence`, `history_undo|redo`, `history_state` event (can_undo/can_redo + i18n labels); destructive sequence stop → commit → new snapshot → reader; clipboard = `Vec<Piece>` in the engine (same document only for now).
- Edit menu + keymap bindings + Undo/Redo labels; waveform refreshes via `audio_rev`.
- Tests: Rust — cut then paste round-trip is sample-identical; delete/trim/silence exact results; undo/redo restores FNV-1a hash; playback stops before commit (fake backend). Vitest — selection pixel↔sample exactness, key bindings dispatch.

## Out (deferred to hardening)
Insert silence dialog, cross-document clipboard + rate conversion, zero-crossing snap, context menu, revision-guarded commands, performance bench at 20 k pieces, journal labels with params (T-301).

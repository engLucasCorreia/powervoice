# H-08 — Export uses the live rack + selection (S4-04 follow-ups)

- **Tier:** Sonnet (no review loop; blocking findings only)
- **Depends on:** S4-04 (export job + dialog), S3-01 (rack panel; `EngineHandle::rack_model()`), S2-01 (selection), S3-06 (NR slot `noise_profile` status).
- **Read first:** CLAUDE.md, MEMORY.md (S4-04, S3-01, S3-06 notes; `/tmp` quota gotcha), src-tauri/src/export.rs (module doc: "rack is always empty"), src-tauri/src/ipc/export_*.rs, ui/src/lib/export/*, ui/src/lib/state/selection.svelte.ts, crates/engine rack_api.rs (`rack_model()`), SPEC-005 export sections, SPEC-014 §"Output noise only".

## Goal
What the owner hears is what the owner exports: effects in the rack (EQ, dynamics, noise reduction, limiter) are rendered into the exported file, and a selection can be exported on its own.

## Scope (in)
- The export job renders through the **current live rack** (`EngineHandle::rack_model()` snapshot taken at export start) instead of `RackModel::default()`, using the existing offline render path (offline = realtime).
- Export dialog: **Whole file / Selection** choice (Selection disabled with no selection); the `range` field is already plumbed end to end.
- SPEC-014: if a Noise Reduction slot in the exported rack has **Output noise only** on, confirm before exporting (Export anyway / Cancel).
- MP3 **VBR** option in the dialog (backend already supports it).

## Tests
- Rust: a live rack with Gain −6 dB → exported peak is 6.00 ± 0.01 dB lower than with an empty rack; a selection export has exactly the selection's length; the exported rack is the one live at export start (later rack edits don't change a running export).
- Vitest: Selection option enabled only with a selection and sends the range; noise-only confirmation shown/blocked/continued; VBR choice reaches `export_start`.

## Out
Export presets beyond ACX, batch export, per-format metadata tags.

`just check` must pass.

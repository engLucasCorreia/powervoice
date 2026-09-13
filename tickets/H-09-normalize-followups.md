# H-09 — Normalize follow-ups (S2-02 / S4-01)

- **Tier:** Sonnet (no review loop; blocking findings only)
- **Depends on:** S2-02 (peak normalize), S4-01 (LUFS normalize), S4-04 (`job_progress` event).
- **Read first:** CLAUDE.md, MEMORY.md (S2-02, S4-01, S4-04 notes; `/tmp` quota gotcha), specs/SPEC-010-normalize.md (deferred ACs), crates/project/src/normalize.rs, src-tauri/src/document.rs (`edit_normalize_peak`, `edit_normalize_lufs`), ui/src/lib/normalize/*.

## Goal
Normalizing a long file shows progress and can be cancelled; the dialog is complete.

## Scope (in)
- Peak and LUFS normalize run as jobs reporting through `job_progress` (new `JobKind` variants) with Cancel; cancel leaves the document untouched (no undo entry).
- Percent targets in the peak normalize dialog (SPEC-010 §2.5); remembered last dialog value/unit.
- Effects menu entry for Normalize… / Normalize to LUFS… (reuse the minimal Effects menu from S3-06).
- Pyramid-accelerated peak scan (SPEC-010 AC-9/AC-15): peaks from the chunk pyramid for untouched chunks, brute force only where needed; result identical to brute force.

## Tests
SPEC-010 AC-9 (timing budget on a large doc, `just test-big` if slow), AC-15 (pyramid = brute force), cancel mid-job leaves hash unchanged, % target math, dialog memory.

## Out
Batch normalize across files.

`just check` must pass.

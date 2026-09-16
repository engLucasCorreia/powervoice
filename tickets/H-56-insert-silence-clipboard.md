# H-56 — Insert silence and the cross-document clipboard (T-302 remainder)

- **Tier:** Sonnet (no review loop; blocking findings only)
- **From:** T-302's row — "S2-01 (done); rest: insert silence, cross-document clipboard".
- **Read first:** CLAUDE.md, MEMORY.md (T-301 document/undo, S2-01 edit ops, H-50 job ordering, T-206 selection, T-701 shortcuts registry + generated docs/shortcuts.md, H-18 fixtures, T-702 i18n lint), specs/SPEC-008 (the edit-ops spec: insert silence and clipboard sections with their ACs), specs/SPEC-004 (undo entries), `crates/project` (edits, clipboard), `src-tauri/src/ipc/document_commands.rs`, the Edit menu.

## Scope (in)
1. **Insert silence** exactly as SPEC-008 defines it: at the cursor or replacing the selection, the length taken the spec's way (a dialog with a duration in the current time format, if that's what the spec says), one undo entry, markers after the insertion point shifted per SPEC-009, and the sidecar/journal updated.
2. **Cross-document clipboard:** copying in one document and pasting into another, within the same app run, per SPEC-008. Sample-rate mismatch handled the spec's way (resample or refuse, don't guess); the clipboard survives closing the source document if the spec says so.
3. Menu entries and shortcuts through the registry (regenerate `docs/shortcuts.md`), i18n keys for every string.

## Tests
- Insert silence: length, marker shifts, undo and redo, at the start, at the end, and with an empty document.
- Clipboard across two documents, including a sample-rate mismatch.
- Applicable SPEC-008 acceptance criteria.

`just check` must pass.

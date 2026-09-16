# H-75 — Editing during a save, and a cancellable overs scan

- **Tier:** Sonnet (blocking findings only)
- **From:** H-70. SPEC-005 §2.7 says editing and playback continue during a save, but the write still holds the document lock, because `History::mark_saved()` records `current_seq()` at call time — releasing the lock would let an edit land mid-write and wrongly clear the unsaved-changes flag. SPEC-005 §2.8 also describes the overs-counting pre-flight as cancellable with progress; it isn't.
- **Read first:** CLAUDE.md, MEMORY.md (H-70's `begin_*`/`finish_*` split and its explicit warning about `mark_saved`; H-30/H-50 job-event ordering; T-301's history and dirty tracking; H-60's save pre-flight parity note), specs/SPEC-005 (§2.7, §2.8), specs/SPEC-018 (dirty tracking), specs/SPEC-004, `crates/project/src/history.rs`, `crates/project/src/save.rs`, `src-tauri/src/document.rs`.

## Scope (in)
1. Add a sequence-targeted `History::mark_saved_at(seq)` (or the equivalent) so the saved point is the snapshot's sequence, not "whatever is current when the write finishes".
2. Take the snapshot under the lock, then release it for the write; an edit made during the write leaves the document correctly dirty, and the file on disk still matches the snapshot.
3. Make the overs pre-flight scan cancellable with progress, per §2.8.
4. Keep everything H-70 established: pre-flight ordering, the busy flag, result-before-Done, the atomic write and the verify step.

## Tests
- An edit during a long save: the document stays dirty, the file matches the snapshot, undo history is intact.
- Cancelling during the overs scan and during the write.
- The existing save suites stay green.

`just check` must pass.

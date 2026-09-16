# H-70 — Save as a real job, with a free-space pre-flight

- **Tier:** Sonnet (no review loop; blocking findings only)
- **From:** H-60's audit. SPEC-005 §4.10 describes Save/Save As reporting progress like the other long operations, and §2.7 step 1 asks for a pre-write free-space estimate. Today saving runs off the main thread but reports nothing, and `error.save.in_progress` exists unused.
- **Read first:** CLAUDE.md, MEMORY.md (H-30 and H-50 — every job service emits its result **and** its error notice before the terminal progress event; T-602's bake service as the model; H-11's `FreeSpaceProvider` and the recording disk floor; H-60's save pre-flight checklist parity note), specs/SPEC-005 (§2.7, §4.10 and the save ACs), specs/SPEC-018, `src-tauri/src/{document,bake,normalize}.rs`, `crates/project/src/save.rs`.

## Scope (in)
1. Save and Save As become a job: `JobKind::Save`, progress events, cancel where the spec allows it, `error.save.in_progress` when one is already running, and the H-50 ordering (result before the terminal `Done`).
2. The progress dialog reuses the existing shared one (the 250 ms delay before it appears, Cancel).
3. **Free-space pre-flight**: estimate the output size and refuse before writing if the volume can't hold it, with the spec's error and a clear message; reuse `FreeSpaceProvider` rather than a new mechanism.
4. Keep `save` and `save_as` pre-flight checklists in sync (H-60's note) and don't regress the atomic write or the verify step.

## Tests
- Progress and cancel behaviour, the busy error, and result-before-Done.
- Pre-flight refusal with a fake small volume; success when it fits.
- The existing save tests stay green.

`just check` must pass.

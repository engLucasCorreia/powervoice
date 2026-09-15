# H-30 — Job-service consistency (after T-602)

- **Tier:** Sonnet (no review loop; blocking findings only)
- **Read first:**
  - CLAUDE.md;
  - MEMORY.md (the T-602 entry: the shared offline render, `begin_bake_job`, `SpectroService::begin_background_job()`, and the notice-before-`Failed` rule);
  - specs/SPEC-007 (tile workers halve during save/export/bake);
  - the job services in `src-tauri/src/{export,loudness,normalize,nr_capture,calibration,bake,document}.rs`.

## Scope (in)
1. **Spectrogram worker guard.** Export and save hold `SpectroService::begin_background_job()` while they run, as SPEC-007 lists them. Tests show the peak running tile workers is at most half during export and during save.
2. **Error ordering.** Audit every job service: each must emit its error notice before the terminal `Failed` progress event, as bake does.
   - Fix the ones that don't.
   - Tests that wait for `Failed` must also find the notice without polling. Add one per service.
   - Also look for other tests that race a later event and fix them the same way.
3. **Rack edits during a bake.**
   - Refuse a backend rack edit while a bake runs, with an i18n'd `error.bake.busy`-style error, instead of silently discarding it when the bake commits.
   - The UI already blocks these with a modal, so this is only the backend guard.
   - Add a test.

## Tests
One or more per item. `just check` must pass.

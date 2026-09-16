# H-50 — Test-infrastructure flakes

- **Tier:** Sonnet (no review loop)
- **Read first:** CLAUDE.md, MEMORY.md (the H-30 rule "every job service emits its error notice before the terminal `Failed` event", the T-602 note about polling instead of racing events, the T-801/T-802 sandbox notes, the T-704 open question about the output callback).

## Scope (in)
1. **`normalize::tests::start_peak_job_runs_end_to_end_and_reports_done_with_a_result`** (`src-tauri/src/normalize.rs`): `wait_for_finish` returns on the terminal `Progress(Done)` and then asserts the `Result` event, which the service emits *after* it. Under load the assertion runs first.
   - Fix the ordering in the **service**, the way H-30 did for failures: emit the result before the terminal success progress event, so anything that sees `Done` already has the result.
   - Audit the other job services for the same success-path ordering (export, loudness, nr_capture, bake, calibration) and fix them together.
   - Add a test per service that asserts the result is present the moment `Done` is observed.
2. **`vox-sandbox-ipc::crash_is_bypassed_and_reported`**: flaked under full-suite load (one block missed its wait budget). Find the real cause; if it's a wall-clock budget, make the test drive the clock or wait deterministically rather than relaxing the budget. Prove it with 30 runs under parallel load.
3. **Output-callback worst case** (T-704's open question): re-run `just bench-callback` on an **idle** machine (no other agents building) and record p50/p95/p99/max per block size in `docs/performance.md`. If the max still exceeds the block deadline, say so plainly and open a follow-up ticket rather than hiding it.

## Tests
One per item, plus the repeated runs named above.

`just check` must pass.

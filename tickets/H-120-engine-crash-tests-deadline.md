# H-120 — The engine's crash tests can hang the same way crash.rs did

- **Tier:** Sonnet (small; test robustness, not product behaviour)
- **Found by H-119:** `crates/engine/tests/punch_crash.rs` (`crash_child` helper, ~line 216) and `crates/engine/tests/recording.rs` (`kill_9_mid_take_recovers_every_appended_sample`, ~line 1316) spawn a child and read its stdout with unbounded `lines.next()` / `for line in stdout.lines()` loops — the hang class H-119 fixed in `crates/project/tests/crash.rs`. Under load, one stalled child blocks until the gate's ceiling kills everything.

- **Read first:** CLAUDE.md, MEMORY.md (H-119), `crates/project/tests/crash.rs` (`LineReader`, `kill_and_reap`, `CHILD_TIMEOUT` — the pattern to copy; it mirrors `crates/plugin-host/src/scan.rs`), and the two tests above.

## Scope (in)
1. Every blocking read from and wait on a child in these two files gets a deadline (same `LineReader`/`kill_and_reap` shape, 20 s), failing with the step, iteration and the child's output so far.
2. Grep the rest of the workspace's tests for any other unbounded child read/wait and fix them the same way; list what you checked.

## Tests
A child that never answers makes the helper fail quickly with the diagnostic, not hang (as H-119's `a_child_that_never_answers_fails_fast_with_a_diagnostic_message`). `just check` must pass. A fresh worktree needs `npm ci --prefix ui` before `just check` (otherwise the notices step misreports).

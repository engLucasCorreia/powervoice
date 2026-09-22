# H-119 — A crash test can hang for as long as the gate allows

- **Tier:** Sonnet (small; test robustness, not product behaviour)
- **Observed twice on 2026-09-21/22:** `just check` was killed by the orchestrator's 50-minute ceiling (signal 15, exit 143) while running `crates/project/tests/crash.rs`. Six of its seven tests printed `ok`; the seventh — `ac9_kill_9_after_success_at_50_points_recovers_the_exact_state` — never finished. Run alone at a load average of 6.5 it passes in **13 s**. Both hangs happened while several agents were compiling in parallel.

## Why it can hang
The test spawns a child process 50 times and, each time, reads the child's output line by line (`run_and_kill`) with **no timeout**. If the child stalls, deadlocks on a pipe, or never reaches the next `go` prompt, the parent blocks forever. A hang is the worst failure mode a test can have: instead of failing in seconds with a message, it consumes the whole budget and takes the entire gate down, with nothing in the log saying why.

- **Read first:** CLAUDE.md, MEMORY.md (T-301's crash tests; H-50 and H-118 on tests that fail under load), `crates/project/tests/crash.rs` (`ChildGuard`, `spawn_child`, `run_and_kill`, and the four tests that use them).

## Scope (in)
1. **Every blocking wait in these tests gets a deadline** — reading from the child, waiting for it to exit — so a stuck child fails the test within a bounded time (seconds, not minutes), with a message saying which step and iteration it was on and what the child had printed so far.
2. **Find out why it stalls under load**, if you can: a pipe filling because stderr is not drained, a race between the kill and the read, a child waiting on stdin that never arrives. Fix the cause if you find one; if you cannot reproduce it, say so and rely on the deadline.
3. Check the other tests in this repo that spawn child processes (`sandbox`, `plugin-host`) for the same unbounded wait, and report what you find.

## Tests
The deadline itself: a test that uses a child that never answers must fail quickly with the diagnostic message rather than hang.

`just check` must pass — and should pass repeatedly with the machine under load.

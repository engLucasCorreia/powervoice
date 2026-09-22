# H-125 — `a_live_plugin_recovery_does_not_mark_the_document_dirty` fails on and off in CI

- **Tier:** Opus
- **Observed:** GitHub CI (`ubuntu`, `just check`) failed on 4 of 8 pushes to main on 2026-09-22 (runs 35728026608, 35726625057, 35724621616, 35689161805) and intermittently on 2026-09-21 — always this test, always `src-tauri/src/document.rs:~7966` "the slot never recovered" (the 5 s deadline waiting for the Missing Gain slot to become Active after `registry.register(GainFactory)`). It passes every time locally, including under heavy load. Read a failing log with `gh run view <id> --log-failed`.

## Why this matters beyond CI
H-40's live recovery (a Missing plugin slot recovers on its own after install/rescan/unblock — the rack polls the registry generation and loads asynchronously) is product behaviour. If recovery can be *missed* rather than merely *slow*, a user who installs a missing plugin sees it stay missing until they reopen the document. Decide which it is before touching the deadline.

- **Read first:** CLAUDE.md, MEMORY.md (H-40, H-119/H-120 on deadlines, H-50/H-118 on load flakes), the test, and the recovery path: who polls the registry generation, on which thread, driven by what tick (does it need the audio callback or a control-thread timer to run? CI has no audio device), and the async load.

## Scope (in)
1. Find the cause. Candidates to rule in/out with evidence: the poll only runs when something else ticks (engine idle on a device-less runner), a generation read before the register that's never re-read, a lost wake-up, the async loader starved on a 2-core runner, or just slowness past 5 s.
2. If it's a real missed-recovery race, fix the product code (real-time rules apply), and add a test that reproduces the race deterministically. If it is only slowness, say why with numbers, and fix the test (bounded wait with diagnostics — what state the slot/generation/loader were in when it gave up).
3. Reproduce the CI conditions locally as far as possible (e.g. `taskset -c 0,1`, no audio device, repeated runs: `cargo test -p powervoice-app a_live_plugin_recovery -- --test-threads=1` in a loop).

## Tests
The fix's test; the existing test passes 200 runs in a row under `taskset -c 0,1`. `just check` must pass.

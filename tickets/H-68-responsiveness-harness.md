# H-68 — UI responsiveness budget (SPEC-002 AC-8)

- **Tier:** Sonnet (no review loop)
- **From:** H-59. AC-8 says no frame may exceed 100 ms while a capture stalls; nothing measures it.
- **Read first:** CLAUDE.md, MEMORY.md (T-704's perf harness and how `just bench-ui` records numbers; H-43's `frameScheduler` and its draw-on-demand rule; H-59's measured-numbers convention and `docs/performance.md` format; H-48's capture-stall handling), specs/SPEC-002 AC-8, `ui/src/lib/render/frameScheduler.ts`, the existing UI benchmark harness, `docs/performance.md`.

## Scope (in)
1. A harness that drives the UI through a simulated 12 s capture stall and records per-frame durations, built on the existing T-704/H-43 tooling — don't invent a second framework.
2. Assert the AC: no frame over 100 ms, plus report p50/p95/max.
3. Record the measured numbers in `docs/performance.md` in the established format, and wire the harness into `just bench-ui` (a fast assertion in `just check` only if it is reliably fast and non-flaky — otherwise keep the measurement in `bench-ui` and put a small deterministic unit test in `check`).

## Tests
The harness itself plus its assertion; no flakiness (run it enough times to be sure and say how many in your report).

`just check` must pass.

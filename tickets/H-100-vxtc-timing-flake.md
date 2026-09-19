# H-100 — A wall-clock assertion in a debug build is flaky

- **Tier:** Haiku (small)
- **From:** H-94 saw `vox-engine tests/rack_api.rs::transfer_curve_encodes_a_vxtc_frame_within_the_time_budget` fail once at **5.303 ms** against SPEC-016 §4.12's 5 ms budget, then pass on rerun. The machine was building several worktrees at the time.
- **The problem:** the test asserts wall-clock time in an **unoptimised** build on a shared machine. That measures the machine's load as much as the code, which is why this repo's convention (MEMORY.md, H-52/H-59) is that real performance numbers come from `just bench`/`docs/performance.md`, not from `just check`.
- **Read first:** CLAUDE.md, MEMORY.md (H-52's bench conventions, H-59's measured-numbers rule), specs/SPEC-016 §4.12 and AC-23, `crates/engine/tests/rack_api.rs`.

## Scope (in)
1. Make the check deterministic: assert the *work* (frame size, point count, allocation behaviour) in `just check`, and move the timing assertion to the benchmark suite where the numbers are recorded and compared against a documented budget.
2. Keep AC-23 genuinely covered — do not simply delete the timing check. Say in your report where the budget is now enforced.
3. Look for the same pattern elsewhere: any other wall-clock assertion inside `just check` has the same flaw. Report what you find; fix the ones your change naturally covers.

`just check` must pass — repeatedly, under load.

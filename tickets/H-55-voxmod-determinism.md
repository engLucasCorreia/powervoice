# H-55 — Packaged-module determinism failure on CI

- **Tier:** Opus (RT determinism through the CLAP wrapper; blocking findings only)
- **Symptom:** `crates/sandbox/tests/voxmod.rs::the_module_test_host_suite_passes_through_the_adapter` fails on GitHub's runner:
  ```
  Tried to (de)allocate memory in a thread that forbids allocator calls!
  ModuleTestHost: `org.powervoice.gain.packaged` failed check `determinism` (seed 0x5eed7005):
  two instances differ at sample 10558 (48000 Hz): Some(-0.0204…)
  ```
  It passes on the development machine: 5 runs, including `--test-threads=1` and under heavy parallel load. The runner is ubuntu-24.04 (2 cores, different CPU) — CI run 35064469629, commit aad42b9.
- **Read first:** CLAUDE.md (RT rules), MEMORY.md (the T-805 entry for `crates/module-clap`'s wrapper: the module lives on the main thread while inactive and moves into the audio processor when active, an atomic parameter mirror answers main-thread calls, sample-accurate events render in sub-blocks; the T-803 CLAP host; H-52's note that the `Tried to (de)allocate` line is `alloc_checks_active()`'s deliberate probe — confirm whether that is what CI printed or whether it is a real violation this time), `crates/module-api/src/test_util/host.rs` (the `determinism` check), `crates/module-clap`, `crates/voxmod-gain`, `crates/sandbox/src/clap/*`.

## Scope (in)
1. **Reproduce it deliberately** rather than by luck: constrain the run the way CI does — `taskset -c 0,1` (2 cores), a debug build, and full parallel load; try several seeds. If it still won't reproduce, add temporary instrumentation to the determinism check that prints both instances' parameter and state timelines, and run it on CI directly (push a branch and read the log) — the branch may stay unmerged until the cause is known.
2. **Find the real cause.** Candidates, in the order the evidence points:
   - a parameter event applied to one instance and not the other (event queue ordering, the "values set while inactive" pre-activation path, or the atomic mirror read at a different moment);
   - state left over between instances (a static, a lazily initialised table, or a pool);
   - denormal flags (FTZ/DAZ) set per-thread, so the two instances run with different flags;
   - uninitialised or reused scratch buffers;
   - a real allocation on the forbidden thread that only happens under CI's timing.
3. **Fix it**, with a regression test that fails without the fix. If the cause is the test harness rather than the wrapper, fix the harness and say so plainly.
4. **Confirm on CI**: the branch's own CI run must pass before you report done.

`just check` and `just check-cross` must pass.

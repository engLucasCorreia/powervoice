# T-110 — Bench harness (divan, callback-time histogram)

- **Tier:** Sonnet (no review loop)
- **Depends on:** T-105 (done). This is the prerequisite for T-704 (performance tuning against targets).
- **Read first:**
  - CLAUDE.md (the `just bench` recipe), MEMORY.md;
  - PROMPT.md, for the performance targets: callback budget, load, memory, UI frame time, open/import/export speed, whichever are listed;
  - specs/SPEC-000 (performance targets, if any), docs/adr/ADR-002 (the RT budget);
  - `crates/engine`, `crates/rack`, `crates/dsp`, `crates/modules`, `crates/project`, and any existing benches.
- **Dependency:** `divan` is named by this ticket. Run `just notices` after adding it.

## Scope (in)
1. **divan benches** (`benches/` in the relevant crates, run by `just bench`):
   - per-module `process()` cost at 48 kHz for 64/128/256/512/1024-frame blocks: gate, NR, EQ, dynamics, limiter, gain, and a full voice rack;
   - LUFS/true-peak measurement throughput;
   - resampler;
   - peaks/overview generation;
   - spectrogram tile computation;
   - chunk-store read/prefetch;
   - export encode (WAV/FLAC/MP3 if LAME is present);
   - sandbox IPC round trip per block (the CLAP test plugin).
2. **Callback-time histogram:**
   - a fake-backend harness that runs the real output callback path with a typical rack for N seconds;
   - it records per-callback wall time into a preallocated histogram (no allocation in the callback; use the `no_alloc` check), then reports p50/p95/p99/max against the block deadline;
   - it is exposed as `just bench-callback`.
3. **Report:** `just bench` writes a Markdown summary to `target/bench/summary.md` with each metric against its PROMPT/SPEC target where one exists. Not committed. This is T-704's baseline.
4. **CI-safe:** benches never run in `just check`, only in `just bench`. A smoke `cargo test` can check that the bench targets compile (`cargo bench --no-run` in `check` only if it's fast; otherwise skip it).

## Tests
- The harness histogram maths.
- The bench targets compile.
- Report the baseline numbers from this machine.

`just check` must pass.

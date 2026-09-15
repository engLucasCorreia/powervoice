# T-704 — Performance tuning against targets

- **Tier:** Opus (blocking findings only)
- **Depends on:** T-110 (bench harness and baseline: `just bench` → `target/bench/summary.md`, `just bench-callback`).
- **Read first:**
  - CLAUDE.md (RT rules), MEMORY.md (the T-110 entry: the baseline shows the rack and callback far under budget; NR's measured cost looks suspiciously low; the H-32 rAF renderers; T-204 spectro workers; T-301 memory budget);
  - **PROMPT.md "Performance targets"**:
    - open a 60-min 48 kHz mono WAV in under 3 s, with the waveform shown progressively;
    - 60 fps scroll/zoom;
    - playback start under 50 ms;
    - full rack CPU and the other targets listed there;
  - specs/SPEC-000 and the spec sections with performance ACs (grep for "ms", "fps", "within").

## Goal
Every PROMPT/spec performance target is measured by an automated check, and met on this machine with a safety margin. Anything that misses is fixed.

## Scope (in)
1. **A targets matrix** in `docs/performance.md`, generated or updated from `target/bench/summary.md` plus the new measurements. For each target: the measured value, the margin, how it's measured, and the command to reproduce it.
2. **Add the missing measurements** as `BENCH_RESULT` lines, following the T-110 convention:
   - **Open time:** 60-min 48 kHz mono WAV → document ready, and time to the first waveform overview; import of FLAC/MP3 if specced. Use the real open path, headless.
   - **Playback start latency:** play command → first non-silent output frame (FakeBackend timing).
   - **UI frame time:** the waveform/spectral render loop at 1280×720 and 2126×850, driven headless. Use the CDP runner pattern from T-708/T-709 (Chrome performance traces or `requestAnimationFrame` timestamps) on `?preview&scene=document` with a 60-minute synthetic document, measuring scroll/zoom frame times (p95 < 16.7 ms).
   - **Memory:** resident memory with a 60-min document open; the memory budget respected (T-301).
   - **NR cost sanity:** confirm the NR bench measures the real FFT path with reduction on, and fix the bench if it doesn't.
3. **Fix whatever misses** or has under 30 % margin. Candidates: progressive peaks, tile scheduling, waveform upload batching, IPC payload sizes, the prefetch ring size. Each fix comes with a before/after number.
4. **Big tests:** add the long-document checks to `just test-big` (release mode), not to `just check`.

## Tests
- Every new measurement produces a `BENCH_RESULT`, and the summary shows pass/fail per target.
- Regression tests for any code change.

`just check` must pass; run `just bench` and `just test-big` for the report.

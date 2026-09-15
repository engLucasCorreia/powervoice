# T-401 — Latency compensation, latency_changed re-activation, A/B dry delay

- **Tier:** Opus (real-time rack; blocking findings only)
- **Audit first.** T-103 already added latency-matched dry paths and the crossfade hold. T-803 and T-806 added plugin latency-change restarts, and T-602 added the offline latency trim. Build a matrix of each spec requirement (implemented, partial or missing), then fill only the gaps.
- **Read first:**
  - CLAUDE.md (RT rules: no allocation, locks or syscalls on the audio thread; drop via the return ring);
  - MEMORY.md (the T-103, T-103 fix round, T-803, T-806 and T-602 entries);
  - specs/SPEC-012 (latency, reporting, compensation, A/B, `latency_changed`, restart and re-activation), specs/SPEC-003 (playhead/latency display, monitoring), specs/SPEC-002 (monitoring latency), specs/SPEC-022 (punch-in latency and calibration);
  - docs/adr/ADR-002, ADR-005 §12 (latency and restart);
  - `crates/rack`, `crates/engine`.

## Scope (in)
1. **Total rack latency.** It is reported and compensated everywhere the spec says:
   - the playhead and meters are aligned to what is heard;
   - the waveform cursor during playback;
   - markers and selection playback;
   - the rack header readout ("1.3 ms (64 samples)");
   - export and bake trim (verify only).
2. **`latency_changed` / re-activation.** When a built-in module changes its latency with a parameter (for example the limiter's lookahead) or a plugin requests a restart:
   - the change is glitch-free: crossfade or hold per SPEC-012;
   - the dry/compensation delays of the other slots are updated without allocating on the audio thread (preallocated maximum or a swapped chain from the control thread);
   - the UI readout updates;
   - tests cover rapid successive changes.
3. **A/B dry delay.** When comparing A/B, or bypassing the whole rack, the dry signal is delayed by the rack latency, so switching doesn't jump in time.
   - The switch uses the spec's crossfade.
   - Bit-exact alignment is tested with an impulse.
4. **Monitoring with latency** (if the spec covers it): input monitoring through the rack reports its added latency and warns above the threshold the spec names.

## Tests
- Impulse alignment through racks with mixed latencies.
- A latency change mid-play: no discontinuity above the spec's threshold, and no allocation (the `no_alloc` harness).
- An A/B switch aligned to the sample.
- The UI readout updates.
- The applicable SPEC-012 acceptance criteria.

`just check` and `just test-big` (if timing tests apply) must pass.

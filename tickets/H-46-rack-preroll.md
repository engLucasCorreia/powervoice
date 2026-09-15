# H-46 — Rack pre-roll on Play/seek

- **Tier:** Opus (real-time engine; blocking findings only)
- **Found by:** T-704, and decided as A-026. With the default voice rack, playback start is **49.4 ms**, just under the PROMPT target (< 50 ms), because the rack's latency (NR 42.7 ms plus the limiter) is added to every start. The target must hold with any realistic rack.
- **Read first:**
  - CLAUDE.md (RT rules), MEMORY.md (the T-401, H-37, T-103, T-602 and T-704 entries: end-of-document drain, playhead history, crossfade/hold, the offline pre-roll policy, the `playback_start` bench);
  - specs/SPEC-003 (play/seek semantics, playback start), specs/SPEC-012 (latency, compensation), docs/adr/ADR-002 (§8 playhead mapping);
  - `crates/engine/src/{output,control,reader}.rs`, `crates/engine/benches/playback_start.rs`.

## Scope (in)
1. **Pre-roll:**
   - On Play and on seek-while-playing, feed the rack the rack-latency's worth of real audio before the play position (from the prefetch ring; the reader seeks earlier by that amount).
   - The heard output starts exactly at the play position with no added delay, so the playback start latency is independent of rack latency.
   - Without a document or before sample 0, pre-roll with silence as today.
   - No allocation or locks on the audio thread.
2. **Correctness:**
   - the playhead mapping (T-401, H-37) stays exact: the first heard sample is the play position;
   - loops and punch-in keep working;
   - export/bake (offline) are unchanged;
   - rack state at the start is warmed up by the pre-roll, as in the offline policy.
3. **Spec:** amend SPEC-003 and SPEC-012 (document the pre-roll and its interaction with seeks during playback).
4. **Bench:** `playback_start` with the voice rack drops to near the empty-rack number. It's asserted as a `BENCH_RESULT` with a margin, and `docs/performance.md` is updated (`just perf-matrix`).

## Tests
- The first heard sample equals the play position, with racks of 0, 480 and 2048+ samples latency.
- Seek during playback.
- Loop and punch-in regression.
- No allocation in the callback.
- Bench before and after.

`just check` must pass; run `just bench` for the numbers.

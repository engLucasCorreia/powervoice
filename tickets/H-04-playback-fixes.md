# H-04 — Playback engine fixes from the S1-01 post-merge review

- **Tier:** Opus (RT audio; orchestrator verifies — no re-review loop)
- **Depends on:** S1-01 (merged). Runs in parallel with S1-04 (which extends `control.rs` for recording) → keep changes localized and minimal.

## Blocking (must fix)
1. **Output fade-out state machine** (`crates/engine/src/output.rs`):
   - END reached during a fade-out drops a queued seek (`end_reached()` ignores `next`) → transport stays `playing`, callback Idle, playhead frozen. Fix: if END arrives during a fade, treat the fade as finished: `begin_wait(e, p, true)` when a seek is queued, else Idle + `Stopped{cmd_epoch, len}`.
   - Play during a Stop fade calls `begin_wait` immediately (click). Fix: a Play during `FadingOut` becomes `FadingOut{next: Some((epoch, pos))}`.
   - Stop during a seek's fade reports the pre-seek position. Fix: Stop during `FadingOut{Some((_, p))}` reports `p`.
2. **Host switch closes the stream without stopping the transport** (`control.rs` `switch_host` → `close_output`). Fix: in `close_output`, when `transport.playing()`, `transport.pause(heard_now(now), false)` + `ReaderCmd::Stop` before teardown (covers every close path).
3. **Rack reset cuts the delayed fade-out when the rack has latency** (`output.rs` 170–172): after a fade-out that leads to a reset (seek, Play with reset), feed `live.latency_samples()` samples of silence (a `Draining{remaining, next}` mode) before `begin_wait`/`live.reset()`. Becomes audible now that the true-peak limiter (latency L + 16) exists.

## Cheap backlog items to include
- Control loop: check the tick deadline after each message (`control::run`), so ticks aren't starved.
- `transport_*` and `telemetry_subscribe` commands: use `spawn_blocking` like the `devices_*` commands.
- Reader: `Detach` calls `release_segments`; if the resampler can't be built, post a notice instead of silently playing at the wrong speed.

## Tests
Fake-backend tests for each blocking scenario (seek in the last 5 ms, Pause+Play within one quantum, Stop during a seek fade, host switch while playing, seek with a 480-sample-latency test module → no output step above the §4.3 criterion); all callbacks under `no_alloc`.

## Deferred (backlog)
Fade before closing on `set_document` rate change / `select_devices`; anchor offset for mid-callback starts; stall-loss playhead cap; underrun fades.

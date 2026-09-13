# H-01 — Rack: bypass during a new instance's warm-up hold + small rack nits

- **Tier:** Opus (RT audio; orchestrator verifies, no re-review loop)
- **Depends on:** T-103 (merged)
- **Source:** T-103 re-review (APPROVE WITH NITS) finding #1 + nits 2, 3, 5; SPEC-012 §2.2/§2.3/§2.9.

## Goal
No click or silent gap when a slot's bypass is toggled (or a slot fails) while a newly inserted/replaced
instance is still inside its latency warm-up hold — needed before Slice 3's latency modules (NR ~40–85 ms).

## Scope (in)
1. Per-slot `warm_left` (samples until the new instance's output and a fresh dry line are valid), counted
   down in `run`. `set_bypass` and `fail()` use `fade_to_after(target, fade_len, warm_left)` instead of
   cancelling the hold. Tests (§4.3 harness, T_s = 15 ms + latency): insert TestDelay(4800) then bypass on
   1000 samples later; insert bypassed then un-bypass inside the hold; restart 480→4800 then bypass inside
   the hold — each passes §4.3 with no silence (reviewer's probe showed 7.6× jumps and 3088 silent samples).
2. `restart()` on a slot that failed to start at load retries resolving the stored slot (SPEC-012 §2.9
   Restart/Remove) instead of `NotLoaded`.
3. In `take_over`, when the mixer is not fading or the old slot isn't found, post `ReplaceDone` so a
   held-back restart can never stay stuck.
4. Count events dropped when `take_over` moves them into a full queue (`dropped_rt_events()`).

## Out
Everything else (latency compensation T-401, presets T-406).

## Definition of Done
Tests first; `just check` green; RT rules kept (`test_util::no_alloc`); report in CLAUDE.md format.

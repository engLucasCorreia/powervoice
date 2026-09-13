# T-105 — Engine core: control thread, RT callbacks, transport, reader/prefetch, resampling, heard clock, telemetry

- **Milestone / wave:** M1 / W2
- **Tier:** Opus (+ Opus review)
- **Depends on:** T-101, T-102, T-103
- **Spec refs:** SPEC-003 (AC-1…AC-4, AC-6…AC-9; AC-5 view part is M2), SPEC-000 AC-2, AC-4, SPEC-004 AC-5 (fake-backend playback part), AC-6 (engine part), SPEC-001 AC-6/AC-7 (transport integration) · **ADR refs:** ADR-002 (all), ADR-003 §1, §3 (`VXTM`), ADR-001 §5

## Goal
Playback works end to end on the fake backend (and on real devices through T-102's cpal backend):
the control thread drives a transport state machine, the reader thread streams the current snapshot
into a prefetch ring, the output callback runs the rack, and a heard-position clock feeds 60 Hz telemetry.

## Scope
**In:**
- Control thread (16 ms tick), lock-free command queue UI→engine, RT event ring (xruns, device lost) engine→control, return ring for retired chains/snapshots.
- Output callback: epochs, ≥ 20 ms pre-buffer, 5 ms fades, sub-blocks of ≤ 1024, rack in path, FTZ/DAZ guard (`vox-dsp`).
- Transport state machine: Play, **Pause (keep position)**, **Stop (return to play-start position)**, **Play from start** (selection start or 0), Return to start, seek, loop with seam reset, engine-initiated stops behave like Pause (SPEC-003 §2.1).
- Reader/prefetch thread (~200 ms) over `vox-project` snapshots; destructive-edit stop → ack → commit → new snapshot sequence (ADR-004 §3).
- Document ≠ device rate: `rubato::Fft` wrapper in `vox-dsp` (`dsp::resample`), primed, delay compensated.
- Heard-position clock from cpal playback timestamps mapped to the app clock; telemetry producer building `VXTM` frames at 60 Hz (channel wiring is T-108).
- Fake-backend integration harness (scripted sessions, random callback sizes, `test_util::no_alloc` around callbacks).

**Handoff requirements from T-101/T-102/T-103 (reviewed and merged — see MEMORY.md ticket learnings):**
- Clock: use the callback timestamps' `now_ns` (app clock) and `to_app_ns()`; never read the clock again in a callback.
- Devices: run `StallDetector` on every control tick and feed a stall into the device state machine as `DEVICE_LOST`; wait for the capabilities diff while `caps_pending` (ALSA ~2.4 s); remember the configured device `id` and make lookup prefer it (twin devices); `StopPlayback` from the device machine must not stop a recording.
- Callback ownership: cpal drops the callback closure (and the `LiveRack` it owns) on its own thread → hand heavy state back through a slot the control thread takes after the stream handle drop returns; use `LiveRack::into_chain` (or the Drop hand-back) so `deactivate()` runs on the control thread.
- Rack: `RackHost::new(registry, cfg, options, model) → (RackHost, LiveRack)`; `live.process(transport, in, out)` in the output callback; `live.latency_samples()` in the heard-position formula; `host.tick()` every 16 ms; add the `dsp::fp` FTZ/DAZ guard to `rack::offline::render` too.
- Document: `SnapshotReader` on the reader thread, `release_segments()` when idle; drop all readers/writers before `Session::close` (which returns the session back in `CloseError` on refusal); destructive edits via `Session` only (it owns the chunk-sync-before-journal rule).

**Out:** recording (T-106), monitoring (T-107), IPC (T-108), UI (T-109), latency compensation beyond the heard-position formula (T-401).

## Crates / files
`crates/engine` (`control`, `output`, `transport`, `reader`, `clock`, `telemetry`), `crates/dsp` (`ftz`, `resample`). Allowed new deps: `rubato` 5 (in `dsp` only), `realfft` not needed yet.

## Acceptance tests to write
- [ ] SPEC-003 AC-1…AC-4, AC-6…AC-9 per its test plan (fake backend).
- [ ] SPEC-000 AC-2 (60 s scripted session, 0 allocations in callbacks) and AC-4 (44.1 k document on a 48 k device → heard position advances 44 100 ± 1 per 48 000 frames).
- [ ] SPEC-004 AC-5 fake-backend part (0 underruns during playback + 200 seeks under a 512 MiB budget) and AC-6 engine part.
- [ ] SPEC-012 AC-1/AC-3/AC-4/AC-9 end-to-end through the fake backend (realtime vs offline ≤ 1e-6).

## Definition of Done
- [ ] Tests first, passing; `just check` green; RT rules verified; report in CLAUDE.md format.

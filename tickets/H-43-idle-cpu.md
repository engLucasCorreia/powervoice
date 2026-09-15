# H-43 — Idle CPU: the web view burns about a full core while idle (owner report)

- **Tier:** Opus (blocking findings only)
- **Owner report (2026-09-15):** the web view's main thread uses close to a full core non-stop while the interface is idle, measured from outside on a debug build. Suspected causes are something redrawing continuously (an animation loop or a fast level meter) or the Rust side sending events too often.
- **Likely causes, to verify rather than assume:**
  - **Perpetual rAF loops.** H-32 made every canvas renderer (waveform, spectral, analyzer, EQ graph) draw from a perpetual `requestAnimationFrame` loop started in `onMount`, so idle views redraw at 60 fps.
  - **Telemetry at 60 Hz while stopped.** The engine sends meter/analyzer telemetry at `telemetry_rate_hz` even when the transport is stopped and nothing changes, and the Svelte stores re-render on every frame.
  - The meters' and analyzer's ballistics, peak-hold and decay animations keep running after the signal is silent.
  - CSS animations or transitions that never finish (spinners, pulsing record button), and `ResizeObserver` or effect loops.
- **Depends on:** H-41 (meter), H-42 (analyzer) and T-704 (performance). Dispatch after they merge, since they touch the same loops.
- **Read first:**
  - CLAUDE.md, MEMORY.md (the H-32, H-16 telemetry rate, H-28 transport, T-708 `themeColors` invalidation, T-110/T-704 performance and bench conventions entries);
  - every `requestAnimationFrame` user in `ui/src`; the telemetry emit path in `crates/engine` and `src-tauri`; the meter, analyzer and spectro components.

## Scope (in)
1. **Measure first.** Record an idle-CPU baseline with a CDP Performance trace (the `?preview` document scene and an empty app state, 10 s idle) in debug and release builds. Report main-thread busy % and the top call stacks (who's drawing, who's receiving events).
2. **Draw on demand.** Keep H-32's robustness (a thrown draw must never stop future redraws), but replace the perpetual rAF loops with a shared scheduler:
   - a renderer requests a frame when its inputs change (data, viewport, theme, size, playhead or telemetry);
   - the loop runs continuously only while something animates (playback, recording, a meter decaying, a zoom animation), then stops;
   - a helper (`ui/src/lib/render/frameScheduler.ts`) with `invalidate()`, and `try/finally` so it always reschedules while active.
3. **Telemetry while idle.** When the transport is stopped and there's no input monitoring:
   - the engine sends telemetry only when values change, or at a low heartbeat rate (for example 4 Hz), falling to silence once the meters hit the floor;
   - while playing, recording or monitoring, it runs at the full `telemetry_rate_hz`;
   - the UI stores skip updates whose values are unchanged, so nothing re-renders.
4. **Meters and analyzer at rest:** decay animations end at the floor and stop scheduling frames, and peak-hold timers don't keep a loop alive.
5. **CSS and effects:** no infinite animations while idle (except an explicit busy spinner while a job actually runs), and no effect or observer feedback loops.
6. **Targets and guard:**
   - idle main-thread CPU in a release build ≤ 2 % of a core, ideally near 0; debug ≤ 10 %;
   - playback smoothness unchanged, still 60 fps while playing;
   - a regression test counts frames and telemetry events over an idle second, in jsdom with fake timers and the scheduler, plus an engine test that telemetry stops or throttles when stopped;
   - add a `BENCH_RESULT` idle-CPU line using the T-110 convention;
   - put the before/after numbers in `docs/performance.md`.

## Tests
- The scheduler stops when idle, and restarts on invalidate or playback.
- Idle telemetry is throttled or stops.
- Meters stop at the floor.
- Playback and recording still animate at full rate.
- A thrown draw still doesn't kill the loop.

`just check` must pass. Don't start or kill the owner's running app; measure with your own headless Vite + chromium, or a separately started app instance on its own data dir if needed.

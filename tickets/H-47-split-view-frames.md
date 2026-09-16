# H-47 — Split-view frame spikes

- **Tier:** Opus (renderers plus tile scheduling; blocking findings only)
- **Found by:** T-704. With the waveform and spectral views shown together (SPEC-007 AC-10):
  - WebGL2: p50 4–7 ms but p99 110–130 ms, with 22–29 frames over 50 ms, clustered on tile arrivals;
  - the Canvas2D fallback runs at 15–34 fps after T-704's per-column fix, still short of 60.
- **Read first:**
  - CLAUDE.md, MEMORY.md (the H-43 renderer rule — every renderer draws on demand from `ui/src/lib/render/frameScheduler.ts`; T-704 measurement tooling `just bench-ui` with its vsync pass, `scripts/bench/ui_frames.mjs`, `docs/performance.md`, `?preview&doc=60min&renderer=`; T-204 spectro service and tile cache; H-32 robustness);
  - specs/SPEC-007 (AC-10 and the tile pipeline), `ui/src/lib/spectrogram/*`, `ui/src/lib/waveform/*`, `ui/src/lib/render/*`, the tile IPC path in `src-tauri`.

## Scope (in)
1. **Find the real cost** in a trace: texture upload per tile, tile decode on the main thread, layout, or GL state. Report it before fixing.
2. **Budget the work per frame:** upload at most N tiles (or N bytes) per frame, spread the rest over later frames, with the newest visible tiles first. Keep the H-43 scheduler contract: draw on demand, animate only while work remains.
3. **Cheaper uploads:** upload only the changed region, reuse textures or an atlas, avoid re-creating GPU objects per tile, and decode off the main thread if that's the cost.
4. **Canvas2D fallback:** reach 60 fps at 1280×720, or document the achievable number and cap the work (for example a lower spectrogram resolution) so it stays smooth.
5. **Re-check the output callback's worst case on an idle machine** (T-704's open question) and record it.

## Tests
- A pure test for the per-frame upload budget and the tile priority order.
- `just bench-ui` numbers before and after, added to `docs/performance.md` as `BENCH_RESULT` rows (p95, p99, frames over 50 ms, for WebGL2 and Canvas2D).

`just check` must pass.

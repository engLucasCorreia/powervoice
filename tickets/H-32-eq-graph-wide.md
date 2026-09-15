# H-32 — EQ graph not drawn at wide window sizes

- **Tier:** Sonnet (bug fix; no review loop)
- **Found by:** the orchestrator, reviewing T-708's screenshots.
  - `?preview&scene=rack` (and `rack-loudness`) at **2126×850**, in every theme: the EQ card shows its grid but no response curve, no band handles and no axis labels.
  - At 1280×720 everything draws.
  - Evidence: `/tmp/claude-1000/-home-lucas-Documents-dev-audition/e6923653-879f-4025-bf4c-c0fb63498c26/scratchpad/t708/shots/{dark,light,hc}/2126/rack*.png` versus `.../1280/rack*.png`.
  - The owner's real app drew the EQ at 2126×850 after a manual resize, which suggests a first-layout measuring bug, not a theme bug.
- **Read first:**
  - CLAUDE.md, MEMORY.md (the T-708, H-26 and H-24 entries);
  - `ui/src/lib/eq/{EqGraph.svelte,axisLayout.ts,freqAxis.ts,gainAxis.ts}`;
  - `ui/src/lib/theme/themeColors.ts`;
  - the rack card and layout (`ui/src/lib/rack/*`, `ui/src/lib/layout/*`).

## Scope (in)
1. Find the root cause. Candidates:
   - a canvas measured at 0 or stale width before the rack column settles, with no ResizeObserver redraw;
   - `axisLayout` returning no labels for some width range;
   - the draw effect not depending on size;
   - a devicePixelRatio or canvas-size limit.
2. Fix it so the graph draws at any rack width and window size, at first paint and after every resize. Check for the same pattern in the analyzer, meters and waveform, and fix any shared cause.
3. Screenshots of `?preview&scene=rack` at 1280×720, 1600×900, 2126×850 and 2560×1440 in Dark and Light. The T-708 CDP runner is in `.../scratchpad/t708/shoot.mjs`. Use Vite `--port 5195 --strictPort`, and stop it when you're done.

## Tests
- A regression test: the pure layout or draw decision at the failing width, and a resize-from-0 path.
- The existing EQ tests stay green.

`just check` must pass.

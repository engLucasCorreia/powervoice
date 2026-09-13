# T-205 — Waveform view: renderer, zoom/scroll, amplitude zoom, rulers, playhead display & follow

- **Milestone / wave:** M2 / W2
- **Tier:** Sonnet
- **Depends on:** T-203
- **Spec refs:** SPEC-006 (all rendering, zoom/scroll, rulers, playhead, follow, clip highlight, PARTIAL display, context loss, HiDPI, performance ACs), SPEC-003 AC-5 (playhead follow) · **ADR refs:** ADR-009 (WebGL2 primary, Canvas2D fallback, DMA-BUF default), ADR-003 §2–§3 · MEMORY A-001

## Goal
The center editor shows the document's waveform exactly as SPEC-006 specifies, smooth at 60 fps on a
60-minute file, with the extrapolated playhead and follow.

## Scope
**In:** WebGL2 renderer with Canvas2D fallback (feature detection, `webglcontextlost` fallback), level selection and ≤ 4 buckets/pixel reduction, raw-sample line + dots at extreme zoom, clip highlight (|x| ≥ 1.0), PARTIAL rendering, stale/out-of-order response rejection, horizontal zoom (`=`/`-`, Ctrl+wheel cursor-centred, zoom to selection/full commands without keys), vertical zoom (`Alt+=`/`Alt+-`, Alt+wheel, 1×–256×), scroll + overview/scrollbar, amplitude ruler (dBFS/%), time ruler (timecode default, samples, seconds), playhead from the T-108 extrapolation module, smooth follow band 10–90 %, marker lines (data from the document; panel is M3), theme tokens, HiDPI; keymap actions registered in the T-104 registry.

**Out:** selection (T-206), spectral pane (T-207), analyzer (T-208), file dialogs (T-209).

## Acceptance tests to write
- [ ] SPEC-006 rendering/zoom/ruler/playhead ACs per its test plan (pure TS functions + Vitest with mockIPC; frame-time ACs measured with the T-007 harness approach in an automated run and reported).

## Definition of Done
- [ ] `just check` green; frame-time numbers in the report; report in CLAUDE.md format.

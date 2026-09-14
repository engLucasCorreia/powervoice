# H-24 — Resizable layout + correct scales, grids and units (owner-reported, TOP PRIORITY)

- **Tier:** Sonnet (UI; blocking findings only) — the owner asked for this ASAP.
- **Owner report (2026-09-14):** "I can not resize the waveform window or anything. It's very bad. The spectrum analyzer is very good, and the meters that jump when I play are also very good. Please fix the scales and grids, with the frequencies and the units correctly."
- **Screenshot of the running app:** `/tmp/claude-1000/-home-lucas-Documents-dev-audition/e6923653-879f-4025-bf4c-c0fb63498c26/scratchpad/powervoice-ui.png` (3401×1360) — read it first.
- **Read first:** CLAUDE.md, MEMORY.md (H-12 shared viewport/ruler/scrollbar in EditorView, `waveformView.svelte.ts`; T-207/H-13 spectral view + renderers; T-208/H-16 analyzer (`analyzerMath.ts`, `spectrum/freqAxis.ts`); S3-07 EQ graph (`eq/freqAxis.ts`, `gainAxis.ts`); H-19 menu bar; S1-03 `bind:this`/`$effect` gotcha; jsdom has no canvas — keep all tick/scale math in canvas-free `.ts` modules; DTO/Settings literal gotcha — new Settings fields need every hand-written TS literal updated; pre-commit `cargo fmt --all --check`), SPEC-006 (waveform rulers, amplitude scale), SPEC-007 §2.1/§2.9/§2.10 (layout, analyzer, shared frequency axis), SPEC-015 graph section, ui/src/App.svelte, ui/src/lib/layout/*, ui/src/lib/waveform/*, ui/src/lib/spectrogram/*, ui/src/lib/analyzer/*, ui/src/lib/eq/*, ui/src/lib/rack/RackPanel.svelte, ui/src/lib/loudness/LoudnessPanel.svelte, ui/src/lib/layout/{MarkersProperties,MeterBridge}.svelte, ui/src/lib/theme/tokens.css.

## Diagnosis from the screenshot
1. The workspace row (Markers | waveform/spectral | Rack) is collapsed to ~20 px; the waveform is a thin strip and the rack panel is almost invisible.
2. The bottom dock (meter bridge + analyzer) fills the window. Causes: `App.svelte` `.shell` has `grid-template-rows: auto auto 1fr auto` for FIVE children (menu bar, toolbar, workspace, LoudnessPanel, bottom-dock — the dock lands in an implicit auto row), and the analyzer canvas apparently grows its container (canvas sized from its own container → feedback loop).
3. No splitters except the waveform/spectral divider; column widths and dock height are fixed.
4. Scales/units: analyzer grid lines are irregular with no frequency labels and no dB axis labels; the waveform has no amplitude scale; the time ruler shows long `00:00:01.000` labels regardless of zoom; the Markers panel header and the Loudness panel's Processed/Source buttons overlap.

## Scope (in)
1. **Layout shell:** explicit rows (menu, toolbar, main area, bottom dock). The main area is a vertical split: workspace (flex, min 40 % of the window) above the bottom dock (default ~240 px, min 120, max 60 %), with a draggable horizontal splitter. The Loudness/ACX panel moves into the bottom dock as a tab or a collapsible section so it can't squeeze the editor.
2. **Resizable columns:** Markers | editor | Rack with draggable vertical splitters (min widths, double-click resets, collapse buttons for the side panels); keyboard-accessible splitters (`role="separator"`, arrow keys).
3. **Persist** column widths, dock height and collapsed states in `Settings` (new `layout` field; Rust `settings.rs` + `just gen-types`), debounced.
4. **No feedback loops:** every canvas (waveform, spectral, analyzer, EQ graph, meters) sits in a container with a definite size and is sized from that container (`position: absolute; inset: 0` or explicit CSS size), never from its own content; resizing the window or a splitter always redraws correctly.
5. **Analyzer scales (keep its look):** log frequency grid at 20, 50, 100, 200, 500, 1k, 2k, 5k, 10k, 20k Hz with labels "20 Hz … 1 kHz … 20 kHz" (minor lines at 30/40/60/70/80/90 etc., lighter); dB axis on the left every 10 dB (or 20 dB when narrow) labelled "0 dB … −120 dB" with the unit (dBFS) shown once; labels never overlap (drop minor labels by available pixels).
6. **Spectral view ruler:** the same shared frequency-tick generator (`spectrum/freqAxis.ts`) for log and linear scales, Hz/kHz formatting; a small colour-bar legend with its dB range.
7. **Waveform:** an amplitude scale on the left gutter (dBFS: 0, −3, −6, −12, −18, −24, −∞ — or % in a toggle), horizontal grid lines at those levels, a zero line; the time ruler adapts to zoom with "nice" steps and compact labels (e.g. `0:05`, `0:05.250`, `1:02:03`) plus minor ticks.
8. **EQ graph axes:** Hz/kHz labels at the standard decades and dB labels (±12/±24), unit shown.
9. **Overlaps:** Markers panel header, Loudness buttons, rack header, toolbar wrap cleanly at 1280 px and at the owner's 2126×850 window; nothing overlaps or clips.
10. The meter bridge and analyzer keep their current behaviour and look (the owner likes them); only their containers and scales change.

## Tests
- Pure tick generators (frequency log/linear, dB, time, amplitude) with exact expected ticks/labels for several widths/zooms; label-collision removal.
- Layout: splitter drag changes sizes within min/max; double-click resets; keyboard arrows move; persisted values restore; the workspace keeps ≥ 40 % height when the dock is dragged; the analyzer container height doesn't change when its content re-renders (regression for the feedback loop).
- Snapshot-free: assert structure via `data-testid`.

## Verification
After `just check`, describe exactly what the owner should see; the orchestrator will screenshot the running dev app after merging.

`just check` must pass.

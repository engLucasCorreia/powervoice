# H-35 — Zoom commands (Zoom to Selection, Zoom Full, vertical zoom)

- **Tier:** Sonnet (no review loop; blocking findings only)
- **Found by:** T-701. SPEC-006 §2.6 says Zoom to Selection and Zoom Full are "implemented and reachable by menu/toolbar", but no such command exists in the code. Vertical (amplitude) zoom, Alt+= / Alt+-, is also unimplemented.
- **Read first:**
  - CLAUDE.md, MEMORY.md (the T-701, T-206, H-27, H-23, H-26 and T-708 entries: shortcuts registry, `ViewportWriter`, Menu, theme tokens, rAF renderers);
  - specs/SPEC-006 §2.6 (zoom) and the SPEC-006 sections on the amplitude ruler;
  - `ui/src/lib/shortcuts/registry.ts`, `ui/src/lib/waveform/*`, `ui/src/lib/state/waveformView.svelte.ts`, `ui/src/lib/layout/ViewMenu.svelte`, the toolbar.

## Scope (in)
1. **Zoom to Selection:** fit the selection to the view with the spec's margin (or none, if the spec is silent). Disabled when there is no selection.
2. **Zoom Full:** fit the whole document.
3. **Vertical amplitude zoom:**
   - Alt+= and Alt+- zoom in and out, Alt+0 (or the spec's binding) resets;
   - both the waveform and the amplitude ruler scale, and the ruler labels stay collision-free (`ui/axisLabels.ts`);
   - the zoom is clamped to the spec's range;
   - it is stored in the sidecar view state if the spec says so.
4. **Wiring:**
   - all commands go through the shortcuts registry, with Audition's bindings where they exist and scopes set;
   - View menu entries use the shared Menu with `Kbd` chips;
   - toolbar icon buttons have tooltips if the spec says so;
   - every viewport write goes through `ViewportWriter`, so playhead and record follow keep working;
   - `docs/shortcuts.md` is regenerated.
5. Correct SPEC-006 §2.6 wherever it doesn't match what ships.

## Tests
- The pure zoom maths: selection fit, full fit, clamping, and the vertical scale mapping.
- Registry and menu chip tests.
- WaveformView integration.
- The ruler label collision check at extreme vertical zoom.

`just check` must pass.

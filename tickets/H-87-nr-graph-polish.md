# H-87 — Noise profile graph: hover readout and the no-output state

- **Tier:** Sonnet (small)
- **From:** H-85's deviations. The graph meets SPEC-014 AC-21, but two §2.8 details were left out because the spec doesn't pin their visual treatment: the **hover readout** (frequency, print level, live level) and **greying the live curve while the analyzer has no output device**.
- **Read first:** CLAUDE.md, MEMORY.md (H-85's entry, and its note that `spectrum/freqAxis.ts` — not `eq/freqAxis.ts` — is the shared axis), specs/SPEC-014 §2.8, `ui/src/lib/rack/NoiseProfileGraph.svelte`, `ui/src/lib/rack/noiseProfilePlot.ts`, and how the Analyzer panel and Spectrum Inspector already present a hover readout (match them rather than inventing a third style).

## Scope (in)
1. Hover readout: frequency plus the print and live levels under the cursor, in the established style, keyboard-reachable if the neighbouring graphs are.
2. A clear "no output device" state for the live curve, consistent with how the rest of the app shows a missing device (H-59 made the device-lost banner tell the truth — match that language).
3. H-85 noted the print and live legend swatches look nearly identical; make them distinguishable.

## Tests
The readout's values against a known print; the no-device state; the legend.

`just check` must pass.

# H-93 — Collision-aware annotation layout

- **Tier:** Sonnet (pure geometry, heavily unit-testable; no UI knowledge needed)
- **From:** the owner's *Explain My Voice* request, §17. Independent of H-91/H-92, so it can be built and tested on its own.

Annotated graphs live or die by whether the labels are readable. This ticket is the layout solver only: given anchor points on a graph and label boxes, place the labels so nothing important is obscured.

- **Read first:** CLAUDE.md, MEMORY.md, `ui/src/lib/ui/axisLabels.ts` (`labelSpan`, `spansOverlap`, `rectsOverlap`, `fitAxisLabels`, `fitGutterLabels`, `estimateLabelWidthPx` — the house collision helpers; extend them rather than writing a third system), `ui/src/lib/analyzer/peakLabels.ts` (the existing peak-label placement, the closest precedent), and the reference image `annotated_voice_spectrum.png` in the repo root for the visual target.

## Scope (in)
1. A pure function: anchors (frequency/level → pixel) + label sizes + plot rect → placed labels with leader lines.
2. Labels must not overlap each other, must not sit on top of the curve's significant peaks where avoidable, and must keep their leader line attached and visible.
3. Labels may move vertically and horizontally; the **anchor** never moves — an arrow must point at the real measured frequency, never at a convenient spot.
4. Priority ordering: when space runs out, the lower-priority labels are the ones dropped (the caller decides priority; the solver honours it and reports what it could not place).
5. Cap the primary annotations (5–7 by default, caller-configurable); the caller shows the rest in a panel.
6. Responsive: the same solver with a smaller rect must degrade to the top 3 without tangling.

## Tests
Overlap-freedom over randomised inputs (property-style: no two placed boxes intersect, every placed box stays inside the rect, every leader line still reaches its anchor), the priority-drop order, the narrow-viewport case, and a snapshot-style test of a realistic voice-spectrum arrangement.

`just check` must pass.

# H-79 — The selection hides the waveform (owner-reported)

- **Tier:** Sonnet, UI/UX quality bar
- **Reported by the owner while using the app:** "The selection color is the same color as the wave graphic. So when I select the waveform I can only see the selection color. Make it another color, and when I select part of the waveform I should still see the waveform inside the selection."

## The bug
`--wave-fill` (`#7fc8ff`) and `--wave-outline` (`#4da3ff`) are the same blue as `--wave-selection-fill`
(`rgba(77, 163, 255, 0.22)`), and `ui/src/lib/render/overlayGeometry.ts` paints the selection rect
*over* the wave content. Inside a selection the wave is a blue shape on a blue wash with almost no
separation, so the user loses the very thing they are selecting. The same overlay path is shared by
the spectral pane (SPEC-007 §4.7), so check both.

- **Read first:** CLAUDE.md, MEMORY.md (H-13's shared overlay geometry; T-708's four themes and `themeParity.test.ts`; H-26's design tokens; `ui/src/lib/theme/contrast.ts` and its AA non-text checks; H-43's frame scheduler), specs/SPEC-006 §2.12 and §4.5, specs/SPEC-007 §4.7, `ui/src/lib/theme/design-tokens.css`, `ui/src/lib/theme/themeColors.ts`, `ui/src/lib/render/overlayGeometry.ts`, `ui/src/lib/waveform/webglRenderer.ts` and the Canvas2D fallback.

## Scope (in)
1. Give the selection its own hue, clearly distinct from the waveform's, in **all four themes**
   (Dark, Light, High Contrast, and whatever System resolves to). Keep it distinct from the other
   overlays too — playhead amber, marker green, loop violet, record red, clip red.
2. **The waveform must stay clearly readable inside the selection.** Pick the approach that looks
   best and say why: draw the selection wash *under* the wave content, and/or draw the wave in a
   selected-state colour inside the selection range. A translucent wash painted over the wave is
   what is broken today — don't just retune its alpha and call it fixed.
3. Keep the selection edges/handles visible against both the selected and unselected background.
4. SPEC-006 §2.12 fixes the token *names*; changing values is free. If you need a new token (e.g. a
   selected-wave fill), add it as a §2.12 amendment in the spec the way earlier tickets did, and
   mirror it in every theme — `themeParity.test.ts` enforces that.
5. Apply the same fix to the spectral pane's overlay if it has the same problem.

## Verification (required)
- `ui/src/lib/theme/contrast.ts` gets checks for the new pairs: selection fill vs wave fill, and
  wave-inside-selection vs selection fill, at the AA non-text ratio.
- **Screenshots**, in every theme, of: no selection, a partial selection over loud audio, a
  selection over quiet audio, and a selection containing markers and the playhead. Put them in
  /tmp/claude-1000/-home-lucas-Documents-dev-audition/e6923653-879f-4025-bf4c-c0fb63498c26/scratchpad/h79/
  and list them in your report. Judge them with your own eyes — the owner will.
- Unit tests for the draw order / selected-range colouring in `overlayGeometry` and the renderers.

`just check` must pass.

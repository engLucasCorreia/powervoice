# H-113 — On the real renderer, a selection hides the spectral view behind flat gray (owner-reported)

- **Tier:** Sonnet, UI/UX quality bar. **Reproduced on the owner's v0.4.0 build — screenshot in the orchestrator's scratchpad (`spectral-gray.png`).**
- **Owner's words:** "When I select the waveform, it selects the Spectral view in gray and I don't see the spectral view at all — it's all gray."

## What the screenshot shows
1. **Spectral pane:** the selected region is a solid, **opaque** gray block — the spectrogram under it is completely invisible.
2. **Waveform pane too:** the selection is a flat **light gray** wash with the wave drawn white on top — **not** the magenta wash H-79 designed (`--wave-selection-fill: rgba(233, 99, 184, 0.28)` in Dark). So the selection colour is wrong in both panes, not only the spectral one.

## The cause behind the cause — a verification gap
**The real app's `renderer_preference` defaults to `Auto` (→ WebGL2 where available) —
`src-tauri/src/settings.rs`. The preview harness defaults to `canvas2d`** (`ui/src/dev/previewIpc.ts`
header: "default canvas2d"). Every screenshot that verified H-79's selection colours, H-92/H-102/
H-104's Explain modal and layout, was therefore taken on the **Canvas2D** path. The WebGL2 path —
the one the owner actually runs — was never looked at. That is how a selection that looks correct in
every screenshot we have ships as opaque gray.

`ui/src/lib/waveform/webglRenderer.ts` (~line 37) says H-79 moved the selection fill to draw *under*
the content on WebGL. Check that the fill's colour and alpha actually reach the shader intact
(token resolution, premultiplied vs straight alpha, blend state), and do the same for the spectral
pane's WebGL overlay — `overlayGeometry.ts` is shared.

- **Read first:** CLAUDE.md, MEMORY.md (H-13's shared overlay geometry, H-79's selection design, T-704's renderer setting, H-43's frame scheduler), specs/SPEC-006 §2.12 and its Amendment 2, specs/SPEC-007 §4.7, `ui/src/lib/waveform/webglRenderer.ts`, the spectral pane's WebGL renderer, `ui/src/lib/render/overlayGeometry.ts`, `ui/src/lib/theme/themeColors.ts`.

## Scope (in)
1. **Fix the WebGL2 path** so the selection renders exactly as H-79 designed, in both panes, in all four themes.
2. **The spectral pane must stay readable through a selection.** A spectrogram *is* its colours, so a wash that works on a waveform may not work here — consider marking the selection by its edges plus a light tint, or a darkening of what is *outside* the selection, and say what you chose. What is unacceptable is any fill that hides the spectrogram.
3. **Close the gap:** verification screenshots from now on must cover **both** renderers. Make that easy — e.g. have the preview harness default to the same renderer the real app does, or make every shoot script capture both. Say in your report which you chose.
4. While in there: the owner's spectrogram rendered as an almost uniform dark red with little visible structure (Inferno colormap, floor −120, ceiling 0). Check whether that is correct for that recording or a WebGL colour-mapping fault of the same kind. Report either way.

## Verification
Screenshots of a selection on **WebGL2 and Canvas2D**, waveform and spectral, Dark and Light, into
the scratchpad `h113/`. The owner's real app is the ground truth — the preview is not, until it
renders the same way.

`just check` must pass.

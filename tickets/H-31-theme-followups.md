# H-31 — Theme and preview follow-ups (after T-708, H-32 and T-709)

- **Tier:** Sonnet (no review loop; blocking findings only)
- **Read first:**
  - CLAUDE.md;
  - MEMORY.md (the T-708, H-32 and T-709 entries; decision A-019);
  - `ui/src/lib/theme/*`, `ui/src/lib/ui/Gallery.svelte`, `ui/src/lib/waveform/*` (the WebGL sample-line path), `ui/src/dev/previewIpc.ts`.

## Scope (in)
1. **A-019.** Match System also follows `prefers-contrast: more`: when the OS asks for more contrast, Match System resolves to High Contrast, and it switches live when the preference changes. Test the resolution logic, including the no-flash boot script in `ui/index.html` (the `boot.test.ts` pattern).
2. **Gallery.** Add a High Contrast column to the component gallery (`?gallery`), next to the Dark and Light columns.
3. **Thick WebGL sample lines.** At the deepest zoom in High Contrast, the waveform's WebGL raw-sample line is 1 px because WebGL can't draw thick lines. Draw it as quads at the theme's line width so it matches Canvas2D. Add a pure test for the quad geometry.
4. **Preview IPC transport commands.** `previewIpc.ts` has no cases for `transport_seek`, `transport_play`, `pause`, `stop`, `play_from_start` and `return_to_start`; they fall through to `null`.
   - Return proper `TransportStateDto` replies built with the H-18 factories.
   - Make an unknown command fail loudly in dev: a console error and a thrown error in tests, so a silent `null` can't hide bugs again.

## Tests
One or more per item. `just check` must pass.

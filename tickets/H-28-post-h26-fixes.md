# H-28 — Fixes left open after H-26 and H-27

- **Tier:** Sonnet (no review loop; blocking findings only)
- **Read first:**
  - CLAUDE.md;
  - MEMORY.md (the H-23, H-26 and H-27 entries);
  - `ui/src/lib/waveform/{WaveformView.svelte,viewportFollow.ts,recordFollow.ts,playbackFollow.ts}`;
  - `ui/src/lib/state/transport.svelte.ts`, `ui/src/lib/layout/ViewMenu.svelte`, `ui/src/dev/preview*`.

## Scope (in)
1. **Recording scene crash.** `?preview&scene=recording` threw Svelte `effect_update_depth_exceeded` from WaveformView's follow effects while a take recorded into an empty document. That was seen before H-27 unified the effects.
   - Reproduce on the current main with a jsdom component test, or with the preview (H-26 used a DevTools-protocol screenshot script; see its MEMORY note).
   - Fix the root cause: an effect must not write state it reads without an untrack or a guard.
   - Add a regression test.
2. **Transport store null guard.** `transport.svelte.ts` throws when telemetry arrives before `transport_get` has resolved (`state` is null, around lines 168 and 206).
   - Buffer the telemetry or ignore it until then. Pick whichever is correct for the real engine.
   - Add a test.
3. **Follow Playhead toggle.** Add a View-menu checkbox bound to `Settings.playhead_follow`, following the pattern of the existing `analyzer_visible` item.
   - Use the shared `Menu` from H-26 (`menuitemcheckbox`) and i18n keys.
   - If SPEC-003/006 names a shortcut, bind it.
4. **Normalize target fields.** The Normalize dialogs' target fields still show an ASCII `-`. Show U+2212 through `units.ts::formatNumber` and parse with its parser, which accepts both.

## Tests
One per item. `just check` must pass.

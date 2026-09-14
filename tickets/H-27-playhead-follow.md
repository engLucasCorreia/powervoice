# H-27 — Playback playhead follow

- **Tier:** Sonnet (no review loop; blocking findings only)
- **Found by:** H-23. SPEC-003 §2.1/§3 and SPEC-006 §2.8 specify playhead follow during playback, but nothing implements it (there is no `follow_band` / `playhead_follow` code).
- **Read first:**
  - CLAUDE.md, MEMORY.md (the H-23 note);
  - specs/SPEC-003 §2.1/§3 and specs/SPEC-006 §2.8 (the follow modes and their settings, if specced);
  - `ui/src/lib/waveform/recordFollow.ts` (H-23's record-head page-flip);
  - `ui/src/lib/waveform/WaveformView.svelte` and `ui/src/lib/state/waveformView.svelte.ts`;
  - the spectral view (it shares the viewport).

## Scope (in)
1. Implement playhead follow during playback exactly as SPEC-006 §2.8 defines it: the continuous band-follow, and any mode or setting the spec names. The view keeps the playhead inside the follow band.
2. A user scroll or zoom during playback suspends following as the spec says. If the spec is silent, follow H-23's rule: resume when the playhead re-enters the view or at the next play.
3. Reconcile with H-23's record-head follow:
   - one follow module with two policies (continuous for playback, page-flip for recording per A-016), or shared helpers;
   - a single viewport-writer hook in WaveformView;
   - no duplicated diffing logic.
4. It works in both the waveform and spectral views, at every zoom level, and in the time-ruler scroll.

## Tests
- Pure unit tests for the band-follow maths:
  - the band edges;
  - a playhead jump (seek) outside the view;
  - loop wrap;
  - the zoom at which the whole document fits, where nothing scrolls.
- User-scroll suspension and its resume rule.
- H-23's `recordFollow` tests stay green.

## Concurrency
H-26, the UI designer, is restyling menus, dialogs and axis labels. Don't restyle anything, and keep the changes to follow logic and its hook.

`just check` must pass.

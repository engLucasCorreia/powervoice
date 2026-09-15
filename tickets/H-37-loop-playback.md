# H-37 — Loop playback

- **Tier:** Opus (engine plus RT; blocking findings only)
- **Found by:** T-401. SPEC-003 AC-4 specifies loop playback, but the engine doesn't implement it.
- **Read first:**
  - CLAUDE.md (RT rules), MEMORY.md (the T-401, H-27, H-28, T-206 and T-701 entries);
  - specs/SPEC-003 (loop: region, wrap behaviour, AC-4, Loop button and shortcut), specs/SPEC-006 (how the loop region and playhead are shown), docs/adr/ADR-002 §8 (playhead mapping and the "at least the play start" clamp);
  - `crates/engine/src/{output,control}.rs`, the reader/prefetch ring (the audio thread never reads the chunk store);
  - `ui/src/lib/state/transport.svelte.ts`, the toolbar Loop button, `ui/src/lib/shortcuts/registry.ts`, `ui/src/lib/waveform/playbackFollow.ts`.

## Scope (in)
1. **Engine loop.**
   - The loop region comes from the spec (the selection when looping starts, or a separate loop range if specced).
   - The wrap is sample-accurate and seamless: the prefetch ring wraps too, so there is no gap or click at the seam. Apply a crossfade only if the spec asks for one.
   - The rack keeps running across the wrap, so tails continue.
   - No allocation or locks on the audio thread.
   - Toggling loop during playback follows the spec.
2. **Playhead across the wrap.** Keep a small, preallocated history of recent (rack input position, output frame) pairs, so the heard position just after a wrap maps back into the loop end for one rack latency, instead of being clamped at the play start (the T-401 note).
3. **UI:**
   - the Loop toggle button with its tooltip;
   - the shortcut from the registry (Audition's, if SPEC-003 names it);
   - the loop region drawn on the waveform, spectral view and ruler with theme tokens;
   - playhead band-follow handles the wrap, so the view jumps back to the loop start.
4. **Export and bake are unaffected by loop.**

## Tests
- Wrap bit-exactness: the output equals the concatenated loop, and the seam shows no discontinuity above the SPEC-003/SPEC-012 click threshold.
- The playhead heard position across the wrap, with rack latency.
- Toggling loop mid-play.
- No allocation in the callback.
- The UI button, shortcut and follow-wrap behaviour.
- SPEC-003 AC-4.

`just check` must pass.

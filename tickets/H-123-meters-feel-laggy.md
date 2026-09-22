# H-123 — The input and output level meters feel laggy and slow

- **Tier:** Sonnet
- **Owner request (2026-09-22):** "The input level bars and output level bar, are very nice. But they are very leggy and slow, why is that? can i setup the speed? between fast and slow? but it looks nice, but maybe could look less leggy". Keep the look; make the motion responsive.

## Scope (in)
1. **Find out why they feel slow**, and report it: the UI update rate (poll/event rate and whether it animates every frame), the ballistics (attack/release time, peak-hold and fall rate), any CSS transition smoothing, and the audio-side metering block/decimation. Quote the current numbers.
2. Make the default responsive: bars follow the signal at display rate (requestAnimationFrame, interpolating between meter updates if they arrive slower), fast attack, a release that reads as smooth but not sluggish. Check the numbers against `docs/references.md` if it has metering standards (e.g. a PPM's integration and fall-back), and say which you follow.
3. **A meter speed setting**, Fast / Medium / Slow (the analyzer already has one — reuse its look and wording pattern), applied to both input and output meters, persisted like the other meter preferences (see how H-112's floor choice is stored). Default: whatever you conclude reads best (likely Medium, or Fast).
4. i18n for every string; docs/help updated where the meters are described (`just help-content`).

## Tests
Ballistics are pure functions: unit-test attack/release per speed (time to fall N dB). A component test that the setting changes the meter's behaviour and persists. `just check` must pass.

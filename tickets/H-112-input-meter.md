# H-112 — The input meter should look and read like the output meter, with a selectable floor (owner request)

- **Tier:** Sonnet, UI/UX quality bar
- **Owner's words:** "The output level bar is very nice and vertical and I can see the audio well when playing. The input level should be the same way and shape — right now it is only a small horizontal bar. It should definitely look like the Output level, but with options to change the mic scale's minimum to −60, −80 or −120, so I can see the level of the mic input well."

## What exists
- **Output:** `ui/src/lib/layout/OutputMeter.svelte` — vertical, fixed size, peak and RMS, peak-hold, clip latch, a readable scale (H-41, H-48). This is the model; the owner likes it.
- **Input:** `ui/src/lib/record/InputMeter.svelte` — a small horizontal peak bar with RMS and peak-hold, fixed at **−60…0 dBFS** (SPEC-002 §2.1), a max readout that resets on click.
- Shared maths already exists in `ui/src/lib/meters/` (`ballistics.ts`, `meterScale.ts`) — reuse it; do not write a second ballistics implementation.

- **Read first:** CLAUDE.md, MEMORY.md (H-41's vertical meter and ballistics, H-48's clip latch), specs/SPEC-002 §2.1 (the input meter's spec — amend it, since the fixed −60 floor is written there), `OutputMeter.svelte`, `InputMeter.svelte`, `MeterBridge.svelte`, `ui/src/lib/meters/`.

## Scope (in)
1. **Same form as the output meter:** vertical, the same size and visual language, peak and RMS, peak-hold, clip indication, the same readable numeric peak/RMS below it. Where the two sit together, they should read as a matched pair — input and output side by side is the natural arrangement for setting a recording level.
2. **A selectable floor: −60, −80 or −120 dBFS**, persisted as a setting. A quiet microphone or a room-tone check is invisible on a −60 floor; −120 shows the noise floor. The scale labels must stay legible at every setting (reuse `meterScale.ts`'s tick fitting).
3. Keep what the input meter already does that the output meter does not: the max readout and its click-to-reset.
4. Amend SPEC-002 §2.1 for the selectable floor, the way earlier tickets amended specs.
5. Do not let a larger meter take space the waveform needs — check the layout at small window sizes.

## Tests
Scale mapping at each floor; the setting persists; peak/RMS/hold behave as the output meter's do; the max-reset still works.

## Verification
Screenshots of both meters side by side, at each floor setting, Dark and Light, into the scratchpad `h112/`. Look at them.

`just check` must pass.

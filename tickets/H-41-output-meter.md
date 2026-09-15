# H-41 — Output level meter: vertical, fixed size, readable (owner request)

- **Tier:** Sonnet, following the design system (`.claude/agents/ui-ux-designer.md` principles; tokens only).
- **Owner request (2026-09-15):** "The output level bar is horizontal now and has a variable size because I think the peak values change the size of the bar, changing the size of the Analyser graphic. Going left and right very fast. Make this Vertical. And the bar is also a bit weird flickering giving a unreadable value. Maybe a filter or a correct readable calculation of the RMS and Peak value would be great." Earlier the owner also said the jumping meters are "very good", so keep the lively bar motion. What has to go is the layout shift and the unreadable numbers.
- **Read first:**
  - CLAUDE.md, MEMORY.md (the H-24, H-25, H-26, T-708, H-32 and H-28 entries);
  - the specs that define the meters (grep `specs/` for "meter", "peak", "RMS", "ballistic" and "hold": SPEC-003 and SPEC-007);
  - the meter components in the bottom dock (Meters tab: "Output level", "Peak … dBFS", "RMS … dBFS");
  - the telemetry path (engine meter values: is the peak a max since the last frame, or an instantaneous sample? is the RMS windowed?);
  - `ui/src/lib/ui/axisLabels.ts`, `ui/src/lib/ui/units.ts`.

## Scope (in)
1. **Vertical meter in a fixed-width column** left of the analyzer. Its width never depends on the values:
   - fixed-width, tabular-numeral readouts;
   - no reflow; the analyzer keeps a stable width. Add a test that the layout doesn't change with the values.
   - Peak and RMS bars side by side, or overlaid in the Audition style (RMS bar inside the peak bar).
   - A dBFS scale with collision-free labels (0, −3, −6, −12, −18, −24, −36, −48, −60, −∞) in true minus.
   - Colour zones from theme tokens: safe, loud, and hot above −3 dBFS.
   - It resizes vertically with the dock (H-24 splitter).
2. **Correct measurement**, done in the engine (or a pure UI module) and tested:
   - peak = the true maximum absolute sample since the last telemetry frame, not a sampled value;
   - RMS over a proper integration window: 300 ms, or the spec's value;
   - dBFS computed correctly, floored at −∞ below the spec's floor.
3. **Readable ballistics:**
   - bar: instant attack, smooth release (about 20 dB/s, or the spec's value);
   - a peak-hold tick held about 1.5 s, then decaying;
   - numeric readouts refreshed at about 4–5 Hz: the peak readout shows the held maximum, and the RMS readout is smoothed;
   - a clip indicator that latches above 0 dBFS (or the spec's threshold) until clicked;
   - `prefers-reduced-motion` respected.
4. The same ballistics helper is reusable by the gain-reduction meters. Adopt it there only if trivial.

## Tests
- Ballistics maths: attack and release, hold, the throttled readout, the clip latch.
- Peak and RMS against synthetic signals: a sine at −6 dBFS gives peak −6.0 and RMS −9.0.
- A meter component test that the layout width is stable across value changes.
- Theme tokens only (the colour-literal scan).

`just check` must pass. Screenshot the Meters tab (the `?preview` scene) in Dark and Light at 1280×720 and 2126×850, saved to the session scratchpad `h41/`.

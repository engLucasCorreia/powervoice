# H-48 — UI polish from screenshots

- **Tier:** Sonnet, following `.claude/agents/ui-ux-designer.md` (tokens only, units everywhere).
- **Read first:** CLAUDE.md, MEMORY.md (the H-41, H-42, T-206, H-26, T-708 and H-43 entries), the screenshots in the session scratchpad `h41/` and `h42/` (view them with the Read tool), `ui/src/lib/layout/{SelectionReadout.svelte,OutputMeter.svelte,MeterBridge.svelte}`, `ui/src/lib/meters/*`, the H-42 peak-marker decay code.

## Scope (in)
1. **Toolbar selection readout.** The Start / End / Length fields clip their values: "00:00:19.(" at 2126 px, and worse at 1280 px.
   - Size the fields to the formatted value in tabular figures for the active time format (timecode, samples or seconds). Nothing may clip at 1280×720 or 2126×850.
   - The toolbar may wrap or collapse per the H-25 toolbar grouping rules.
2. **Output meter:**
   - Scale labels crowd at short dock heights (−12/−18/−24 run together). Use `axisLabels` fitting to drop labels that don't fit.
   - The bar is thin compared with its 13 rem column: make it wider and more readable, with the RMS fill inside the peak fill.
   - Keep the fixed column width, so the analyzer doesn't resize.
3. **Peak-marker decay.** H-42's peak-hold markers use their own decay. Move them onto H-41's shared `PeakBallistics` (hold, then release, then snap to −∞), so there is one ballistics implementation and the marker animation stops at rest (idle CPU).
4. **Screenshots:** the Meters tab with a document and the toolbar with a selection, in Dark and Light at 1280×720 and 2126×850, saved to the session scratchpad `h48/`.

## Tests
- A layout test that the readout doesn't clip, for each time format.
- Meter label fitting at small heights.
- The marker decay goes through `PeakBallistics` and reaches rest.

`just check` must pass.

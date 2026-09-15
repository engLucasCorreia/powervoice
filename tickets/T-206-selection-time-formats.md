# T-206 — Selection model, time formats, zero-crossing snap

- **Tier:** Sonnet (no review loop; blocking findings only)
- **Depends on:** T-205 (done). Much of the selection already exists from the vertical slices, so **audit first**: build a matrix of each spec requirement (implemented / partial / missing) and fill only the gaps.
- **Read first:**
  - CLAUDE.md, MEMORY.md (the H-23, H-27, H-28, H-26 and H-18 entries);
  - specs/SPEC-006 (waveform view: selection gestures, time display and formats, the zero-crossing snap setting), specs/SPEC-008 (edit ops on the selection, zero-crossing rules), specs/SPEC-009 (marker snapping), specs/SPEC-004 (history interactions), specs/SPEC-003 (shortcuts and settings);
  - `ui/src/lib/waveform/*`, `ui/src/lib/state/*` (selection/transport/waveformView), `ui/src/lib/ui/units.ts`, the time display in the toolbar, `crates/*` for any backend zero-crossing search.

## Scope (in)
1. **Selection model** as the spec defines it:
   - click-drag, shift-extend, double-click word/region if specced, select all/none;
   - keyboard nudge and extend, selection across the spectral view;
   - selection start/end/length readouts that you can edit, with units;
   - the interplay with the playhead and cursor;
   - persistence in the sidecar view state if specced.
2. **Time formats:** every format the spec lists (e.g. `hh:mm:ss.mmm`, samples, seconds, SMPTE/frames if listed), switchable from the time display and View.
   - One formatter/parser module in `units.ts`, or next to it, with round-trip tests.
   - Everything that shows time uses it: toolbar clock, ruler, marker list, selection readouts, dialogs.
3. **Zero-crossing snap:**
   - selection edges, and cut/delete/insert points if specced, snap to the nearest zero crossing within the spec's window, when the setting is on;
   - the backend search reads the audio off the audio thread (never the RT path);
   - the setting and its toggle go through the shared Menu and `Settings`;
   - update `test/fixtures.ts` for any new field.

## Tests
- Pure tests for the selection maths, each format's format/parse round trip, and the zero-crossing search on synthetic signals (DC, a sine, noise, silence, a window with no crossing).
- UI interaction tests.
- The spec's acceptance criteria that apply.

`just check` must pass.

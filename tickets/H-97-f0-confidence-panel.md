# H-97 — Show pitch confidence in the diagnostics panel

- **Tier:** Haiku (small, contained)
- **From:** H-91, which added `F0StatsDto.confidence` and `.octave_corrected` but deliberately didn't touch the existing panel.
- **Why it matters:** on the owner's own recording, 17.7 % of voiced frames needed octave correction. The numbers the panel shows are now trustworthy, but nothing tells a user when an estimate was shaky.
- **Read first:** CLAUDE.md, MEMORY.md (the H-91 entry), `crates/dsp/src/diagnostics/f0_profile.rs`, `ui/src/lib/analyzer/DiagnosticsPanel.svelte`, `ui/src/lib/analyzer/diagnosticsHints.ts`.

## Scope (in)
1. When `confidence` is low (pick and document a threshold), mark the F0 readout as an estimate rather than showing a number that looks as solid as a clean one.
2. Do **not** show `octave_corrected` as a raw percentage to users — it is a diagnostic of our own tracker, not a property of their voice. If it is worth surfacing at all, it belongs in a tooltip or the hover detail, worded for a human.
3. Keep the panel's existing layout and restraint; this is one small signal, not a new section.

## Tests
The low- and high-confidence renderings; the threshold boundary.

`just check` must pass.

# H-42 — Analyzer diagnostics: peaks, voice statistics, Spectrum Inspector (owner request)

- **Tier:** Opus, in the `ui-ux-designer` role (`.claude/agents/ui-ux-designer.md`) plus DSP. Blocking findings only.
- **Owner request (2026-09-15):** "The analyser graphic is very beautiful. Give more details on it. Like showing the highest frequency peak values on the graph itself, cool diagnostic statistics, so I can find out my voice or the audio frequencies like my frequency for de-esser, or my main frequencies, maybe I need an additional FFT window that I can activate or deactivate? Be creative."
- **Constraint:** keep the analyzer's current look (the owner loves it). Everything new is an overlay or a panel that can be toggled.
- **Concurrency:** H-41 is changing the output meter column next to the analyzer at the same time. Don't touch the meter; stay inside the analyzer panel, new diagnostics components and the DSP analysis code.
- **Read first:**
  - CLAUDE.md (RT rules: analysis never runs on the audio thread), MEMORY.md (the analyzer entries, H-24 dbAxisTicks, H-26 axisLabels and Menu, T-708 themeColors and tokens, H-32 rAF renderers, T-204 spectro service, S4-01 loudness job service, T-110 benches);
  - specs/SPEC-007 (the analyzer), specs for the EQ (to add a band);
  - `ui/src/lib/analyzer*`, the analyzer telemetry path, `crates/dsp` (FFT and spectro), the loudness/NR job services (the offline analysis pattern).

## Scope (in)
1. **Peak labels on the graph** (toggle):
   - the top N spectral peaks (default 5, minimum spacing, above the floor) marked on the curve with a small label: frequency, musical note with cents (e.g. "A3 +12¢ · 220 Hz"), and level;
   - peak-hold markers that decay slowly;
   - a hover crosshair showing frequency, dB and note at the cursor;
   - labels never collide (use `axisLabels`-style fitting) and use theme tokens.
2. **Voice diagnostics panel** (toggle; a side panel or strip under the graph). Live values, smoothed, plus "over the last N s":
   - **Fundamental (F0):** pitch estimate (YIN or autocorrelation) with median and range over the last N seconds, shown as note and Hz. It's descriptive only; no voice-type labels.
   - **Sibilance:** the 4–10 kHz band energy relative to the overall level, and the dominant sibilance frequency → "De-esser target ≈ 6.3 kHz".
   - **Tone balance:** mud/boxiness (200–500 Hz), presence (2–5 kHz) and air (10–16 kHz) relative to the 1 kHz reference, shown as simple bars with plain-language hints ("a bit boomy", "presence OK").
   - **Hum:** 50/60 Hz and their harmonics standing out → "Mains hum at 50 Hz (+harmonics)".
   - **Rumble:** energy below 80 Hz → "Consider a high-pass at 80 Hz".
   - **Noise floor and SNR:** from the quietest windows; tie into the ACX noise floor definition if it matches.

   Each finding offers **"Add EQ band here"**: insert or configure a Parametric EQ band in the rack (a notch for hum, a peak cut for mud or a harsh frequency, a high-pass for rumble) through the normal rack commands, so the change can be undone the rack's way. The de-esser suggestion only copies the frequency, since there's no de-esser module yet.
3. **Analysis modes** (a segmented control):
   - **Live:** the current behaviour.
   - **Long-term average (LTAS)** over the selection or the whole document, computed offline by a job in Rust off the audio thread, with a progress bar and cancel. This is the real "my voice" curve.
   - **Freeze/Compare:** snapshot a curve and overlay it (A/B), including Source vs Processed (dry document vs through the rack).
4. **Spectrum Inspector window** (View → Spectrum Inspector, toggle, plus a dock tab or a larger panel):
   - FFT size 1024–32768 and window function;
   - fractional-octave smoothing (none, 1/3, 1/6, 1/12);
   - log or linear frequency axis, and zoom into a frequency range;
   - the same peak labels and diagnostics at higher resolution;
   - export the current curve as CSV.
5. **Performance:** the live analysis stays within frame budget. Heavy work runs in a Web Worker or the Rust backend; the rAF loop only draws. Add a bench line (T-110 convention) for the analysis cost per frame.

## Tests
- Peak picking: synthetic multi-sine peaks found with the right frequency and level, and the minimum spacing respected.
- Note naming and cents.
- F0 on a sine, a harmonic series with a missing fundamental, and a voice-like signal.
- Sibilance frequency on band-limited noise.
- Hum detection at 50 and 60 Hz with harmonics.
- LTAS equals the averaged spectrum on a known signal.
- "Add EQ band" issues the right rack command.
- UI toggles, keyboard access and label collision.
- Screenshots of every mode and panel in Dark and Light at 1280×720 and 2126×850, in the session scratchpad `h42/`.

`just check` must pass. Every string goes through i18n; the T-702 lint must stay green.

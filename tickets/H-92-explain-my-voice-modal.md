# H-92 — "Explain My Voice": the modal and the annotated graph

- **Tier:** Sonnet, UI/UX quality bar. **Depends on H-91** (engine) and **H-93** (layout solver) — read both before starting; do not duplicate their work.
- **Owner request, in their words:** the modal "should feel like a professional audio-engineering tool, not an AI-chat popup". Reference image: `annotated_voice_spectrum.png` in the repo root.

## Scope (in)
1. **The button.** *Explain My Voice*, near the existing spectrum controls (Analyzer panel and Spectrum Inspector — pick the right home and say why). It freezes the current snapshot (H-91) and opens the modal; closing returns to live analysis, which must keep running underneath untouched.
2. **The modal**: title *Voice Spectrum Analysis*, subtitle naming the real analysed span (from `VoiceReport::span_s` — never a hardcoded "10 seconds"). Large, responsive, dark/light/high-contrast correct, built on the shared `Dialog`/kit, Escape-closable, focus-managed.
3. **The graph**:
   - **Logarithmic** frequency axis, 20 Hz → 24 kHz, using the shared `spectrum/freqAxis.ts` so it aligns with the rest of the app. A linear axis is a fail.
   - dBFS level axis over the measured range, never truncating real peaks.
   - **Two curves**: raw FFT (thin, low opacity, secondary) and the fractional-octave smoothed envelope (thick, dominant) via the existing `analyzer/smoothing.ts` (~1/12 octave default).
   - F0 marker, supported harmonics H1…H6, and the strongest-peak marker distinguished from F0.
   - Subtle voice-region bands (rumble / fundamental / low-mids / midrange / presence / sibilance / air), overlapping where acoustically right, never overpowering the curves.
   - Annotation cards with leader lines to their **real measured** anchors, placed by H-93's solver.
4. **Toggles**: Raw FFT, Smoothed, Harmonics, Voice Bands, EQ Advice — defaults as the owner specified (raw on but subtle, smoothed dominant, the rest on).
5. **Hover readout**: frequency, raw level, envelope level, and the region name when inside one. Match the existing `SpectrumPlot` hover style; keyboard-reachable like the other graphs.
6. **Responsive**: desktop wide with annotations around the graph; mobile shows the top three annotations on the graph and the rest as cards beneath.

## Verification (required)
Screenshots in Dark, Light and High Contrast, at desktop and phone widths, into the session scratchpad `h92/` — **capture the window contents, not just the fact that a window exists** (`hyprctl clients -j` for geometry, `grim -g`). Judge them yourself against the reference image and say honestly how they compare.

`just check` must pass.

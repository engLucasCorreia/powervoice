# H-91 — "Explain My Voice": the analysis engine and data model

- **Tier:** Opus (the reasoning here is what makes the feature credible or embarrassing)
- **Owner request:** an *Explain My Voice* button on the spectrum analyzer that freezes the current analysis and explains the user's voice the way an experienced engineer would — every number measured, nothing hardcoded. This ticket builds the **engine only**; H-92 draws it, H-93 lays out the labels, H-94 writes the prose.

## What already exists — reuse it, don't reimplement
- `crates/dsp/src/diagnostics/` — `voice.rs` (`VoiceReport`), `features.rs` (`tone_balance` mud 200–500 Hz / presence 2–5 kHz / air 10–16 kHz relative to a reference where pink noise reads 0 dB; `sibilance` **with its centre frequency**; `detect_hum` over harmonics 1…8 of 50 and 60 Hz with a prominence test; rumble), `pitch.rs` (YIN, `Pitch { f0_hz, aperiodicity }`).
- `VoiceReportDto` already carries `f0 { current_hz, median_hz, low_hz (10th pct), high_hz (90th pct), voiced_fraction }`, `tone`, `sibilance`, `hum`, `rumble_db`, `noise_floor_dbfs`, `active_level_dbfs`, `snr_db`, `span_s`.
- **The noise floor is already a broadband time-domain RMS** (minimum of 10 ms mean-square blocks over a window — `voice.rs::level_stats`), *not* an FFT-bin floor. Use it, label it as such, and **never** derive a floor from FFT bins.
- UI: `analyzer/smoothing.ts` (fractional-octave **power-mean** smoothing on a log axis — already exactly what the request asks for), `analyzer/notes.ts` (note + cents, A4 = 440), `analyzer/peaks.ts`, `spectrum/freqAxis.ts` (the shared SPEC-007 §2.10 log axis), `analyzer/analyzerMath.ts`, `analyzer/eqSuggest.ts`, `ipc`: `analyzer_voice_subscribe` (live), `spectrum_analyze_start`/`_curve` (offline over a selection), `spectrum_export_csv`.

## Scope (in)
1. **A frozen snapshot**: one immutable analysis object built from the current analyzer state — the spectrum (frequencies, raw dB, smoothed dB) plus the existing `VoiceReport`. Computed **once** per click, memoized; never recomputed at frame rate.
2. **Harmonic analysis**: from the robust median F0, the expected H1…H6, and for each whether the spectrum actually supports it (level, and how close the nearest real peak is). Only report harmonics the data supports.
3. **Strongest peak vs fundamental**: find the strongest spectral peak and decide whether it *is* the fundamental or a harmonic of it. When the strongest peak is Hn (n > 1), that is a headline finding — this is the single most educational thing the feature does.
4. **Octave-error protection**: a pitch tracker reporting F0/2 or F0×2 for a few frames must not move the displayed profile. Use the existing aperiodicity as confidence plus harmonic consistency; prefer a robust median of confident voiced frames. Say in your report how you tested this.
5. **A findings model**: each finding carries category, severity (`info` | `good` | `attention` | `significant`), the frequency or range it anchors to, the measured value, and a priority. Thresholds live in one documented place, not scattered across the UI.
6. **Severity discipline**: a deep voice with strong 200–500 Hz is `info: strong body`, not a problem. Escalate only when a measured threshold is actually crossed. Measurement, classification and interpretation stay three separate things in the model.

## Out
Drawing, layout, prose, the EQ overlay (H-92/93/94).

## Tests
Property and fixture tests for harmonic detection and the peak-vs-F0 decision, the octave-error guard, severity thresholds at and either side of their boundaries. Use the owner's `spectrum.csv` in the repo root as one real fixture. **No test may assert a hardcoded example frequency from the request** (100 Hz, 4.8 kHz…) — those are illustrations, not data.

`just check` must pass.

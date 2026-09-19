# H-94 — "Explain My Voice": the words

- **Tier:** Opus (the wording is where this feature earns or loses an engineer's trust). **Depends on H-91.**
- **Owner request:** every annotation carries category, measured value, interpretation and an optional recommendation; the summary is generated from real measurements; the tone stays technical, never alarmist.

## The rule that governs everything here
Separate **measurement** from **interpretation**, always.

> Bad: "Your voice is boomy."
> Good: "Low-mid energy is elevated +10.8 dB relative to the 1 kHz reference. This may contribute to warmth or boominess depending on microphone distance and processing."

## Scope (in)
1. **Finding prose**: for each finding from H-91, a title, the measured value, an interpretation, and a recommendation only where the measurement justifies one. All strings through i18n.
2. **The engineering summary**: a voice profile (median pitch with note, body, presence, sibilance, rumble, hum) plus a short "suggested focus", composed from the real numbers.
3. **Conservative recommendations only**: no static suggestion beyond roughly ±3 dB; prefer "check microphone proximity / LF enhancement" before EQ; a de-esser is suggested only when the measured severity warrants it, starting at the **detected** centre frequency.
4. **Principles that must hold in the generated text** (these are the owner's, and they are correct):
   - a voice spectrum does not need to be flat, and high-frequency roll-off is normal — never advise boosting air to flatten it;
   - the fundamental need not be the strongest harmonic;
   - proximity, compression, microphone response and room all shape what was measured, so interpretation is possibilistic, not diagnostic;
   - "harsh" and similar words appear only when the evidence is substantial.
5. **Noise-floor honesty**: report the broadband RMS floor and SNR from `VoiceReport` and label them as such; if only a spectral estimate were ever available it must be labelled "FFT spectral floor estimate" and must not be used for an ACX-style claim.
6. **The clean-recording case**: when nothing is wrong, say so. The feature must not invent a finding per section to look busy.

## Optional EQ overlay
A dashed suggestion curve drawn over — never modifying — the measured spectrum, reusing `analyzer/eqSuggest.ts` where it fits, and applyable through the existing `eqApply.ts` path if that is a natural fit (say what you chose).

## Tests
Golden-text tests over several synthetic reports (balanced voice, elevated low-mids, strong sibilance, hum present, clean recording): assert the wording that must appear and, just as importantly, **assert that alarmist or absolute phrasing never appears** for a balanced measurement.

`just check` must pass.

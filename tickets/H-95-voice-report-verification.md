# H-95 — "Explain My Voice": verification against real voices

- **Tier:** Sonnet. **Runs last**, after H-91/92/93/94.
- **Why:** every number this feature shows is a claim about the user's voice. The owner will place a real recording in the project folder for exactly this.

## Scope (in)
Work through the owner's own test list against the built feature, using real or synthesised audio, and report what the feature actually says for each:
- **A.** A low male voice ~90–110 Hz **whose H2 is stronger than F0** — the fundamental must still be identified correctly, and the strongest-peak finding must explain the difference.
- **B.** A higher voice ~180–250 Hz — nothing may assume a low male voice.
- **C.** Strong 50 Hz mains hum with harmonics — detected, with arrows at the real harmonic frequencies.
- **D.** Heavy proximity effect — elevated low-mid energy, with the proximity/LF-enhancement advice *before* any EQ suggestion.
- **E.** Strong sibilance around 6–8 kHz — the suggested de-esser frequency must move to the detected centre, not sit at a default.
- **F.** A very clean recording — **no invented findings.**
- **G.** A quiet recording with poor SNR — noise analysis works from the broadband RMS floor, independent of the FFT bins.
- **H.** A pitch tracker octave error — a short false reading must not redefine the median profile.

**The owner's own recording is in the repo root: `ExampleRecording.wav`** — mono, 48 kHz, 24-bit,
31.5 s of their real voice. Open it in PowerVoice, run the feature on it, and include a screenshot
of the modal plus every number the feature reports. Their exported `spectrum.csv` (8 186 points,
`frequency_hz,level_db`) is the same voice as a spectrum and is the natural fixture for the pure
engine tests. **Sanity-check the feature's output against that CSV independently** (compute the
band levels and the strongest peak yourself from the CSV and compare) — if the feature and the raw
data disagree, the feature is wrong.

This recording is the owner's own voice: treat its analysis as the acceptance case for the whole
feature, not as one test among eight.

## Deliverable
A written report: for each case, what was fed in, what the feature said, and whether an audio engineer would consider the interpretation defensible. Fix what is wrong; where a fix belongs in H-91/92/94's code, make it there rather than papering over it in a test.

`just check` must pass.

## Independent ground truth from the owner's `spectrum.csv`

Computed by the orchestrator straight from the CSV (8 186 points, 2.93 Hz bins, 20.5 Hz – 24 kHz),
summing power per band, **before** any of this feature existed. The feature's own numbers must be
explainable against these; where a convention differs (the app's band levels are normalised so pink
noise reads 0 dB, these are raw band power and a bandwidth-normalised comparison against the
707–1414 Hz octave), the *relationships* must still agree — and any disagreement in sign or
magnitude means the feature, not this table, is wrong.

| Measure | Value from the CSV |
|---|---|
| Strongest bin | **199.2 Hz at −31.3 dB** |
| Next strongest low peaks | 170 Hz/−39.2, 114 Hz/−42.7, 135 Hz/−43.0, 146 Hz/−43.3, 229 Hz/−43.6, 302 Hz/−44.6 |
| Sub-80 Hz band power | −53.0 dB |
| 80–200 Hz | −24.0 dB |
| 200–500 Hz | −26.9 dB |
| 500 Hz–2 kHz | −39.4 dB |
| 2–5 kHz | −46.9 dB |
| 4–10 kHz | −58.0 dB |
| 10–16 kHz | −67.9 dB |
| 200–500 Hz vs the 1 kHz octave (bandwidth-normalised) | **+15.5 dB** |
| 2–5 kHz, same basis | **−4.5 dB** |
| 10–16 kHz, same basis | **−22.5 dB** |
| 50 Hz vs its neighbours | +1.5 dB (100 Hz −7.6, 150 Hz −1.8) |
| 60 Hz vs its neighbours | −2.0 dB (120 Hz +0.2, 180 Hz −3.7) |

**What this voice is, in plain terms** — the feature should reach these same conclusions from its
own measurements, in its own words:
- the strongest peak is **not** the fundamental (a ~100 Hz voice with a dominant H2 at 199 Hz);
- low-mid/body energy is genuinely elevated — this is the headline tonal finding;
- presence sits slightly below the reference: balanced-to-recessed, **not** harsh;
- the air band rolls off normally — nothing to fix;
- **no mains hum**: no harmonic stands above its neighbours convincingly, so a hum finding here
  would be a false positive and a test must guard against it.

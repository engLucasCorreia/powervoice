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

Also check the owner's `spectrum.csv` and any recording they leave in the repo root; if a recording is present, run the real feature on it and include a screenshot of the modal.

## Deliverable
A written report: for each case, what was fed in, what the feature said, and whether an audio engineer would consider the interpretation defensible. Fix what is wrong; where a fix belongs in H-91/92/94's code, make it there rather than papering over it in a test.

`just check` must pass.

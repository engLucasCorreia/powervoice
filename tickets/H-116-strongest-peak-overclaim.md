# H-116 — Explain My Voice calls an unresolvable harmonic "a room resonance" (found in the owner's screenshot)

- **Tier:** Sonnet. **A correctness bug in what the feature tells people** — the one failure it must never have.

## What happened
On the owner's second take the report said:

> "The loudest peak in the spectrum is at 215.6 Hz, −31.4 dB. It does not line up with any harmonic
> of the measured pitch range, so it is a resonance of the voice or of the room rather than a partial."

The take's pitch range was **76–121 Hz** (median 106 Hz). `highestSeparableHarmonic` returns
`floor(1 / (121/76 − 1)) = 1` — only H1 can be separated, because H2's band (152–242 Hz) runs into
H3's (228–363 Hz). So `relateStrongestPeak` correctly **declines to name** a harmonic number…

…but **215.6 ÷ 2 = 107.8 Hz, squarely inside the measured pitch range.** The peak is almost
certainly H2. The prose then converts "cannot be resolved" into "is not a harmonic — it is the
room", which is a confident and probably false statement. On the owner's *first* take the same
engine correctly reported H2; the only difference is that this take's pitch moved further.

- **Read first:** CLAUDE.md, MEMORY.md (H-91 — "a harmonic of a speaking voice is a band, not a line"; H-94 — measurement separated from interpretation; H-95's verification), `ui/src/lib/analyzer/explain/harmonics.ts` (`highestSeparableHarmonic`, `harmonicIsResolvable`, `relateStrongestPeak`), `ui/src/lib/analyzer/explain/prose.ts`, and the `explain.finding.strongest_peak.*` strings in `en.json`.

## Scope (in)
1. **Three states, not two.** Distinguish:
   - the peak *is* harmonic n (resolvable, and it lines up) — today's behaviour;
   - the peak **lines up with a harmonic that cannot be separated in this take** — say so: which harmonic it most likely is, the implied fundamental and that it falls inside the measured range, and that the pitch moved too much to be certain;
   - the peak lines up with **no** harmonic of any pitch the speaker actually used — only then may it suggest a resonance, and even then possibilistically.
2. **Never assert "resonance of the room"** without that last case being true.
3. Check the other findings for the same shape of error: anywhere "could not be measured" is turned into a negative claim.

## Tests
The owner's case exactly (range 76–121 Hz, peak 215.6 Hz) must produce the second state; a peak whose implied F0 falls outside the range for every n must produce the third; the resolvable case must be unchanged. Golden-text tests assert that **"rather than a partial" and "resonance of the room" never appear** in the second state.

`just check` must pass.

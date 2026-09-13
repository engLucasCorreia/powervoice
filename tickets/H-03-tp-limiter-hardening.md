# H-03 — True-peak limiter hardening (S3-05 follow-ups)

- **Tier:** Opus (DSP/RT; blocking findings only)
- **Depends on:** S3-05 (TruePeakLimiter module), S3-01 (rack panel for the GR meter), S4-01 (`vox_dsp::loudness` true-peak reference).
- **Read first:** CLAUDE.md, MEMORY.md (S3-05 learnings incl. the measured +0.011 dB bound; T-103 §4.3 harness; RT rules), specs/SPEC-017-true-peak-limiter.md (full AC list), crates/modules/src/true_peak_limiter*.rs, crates/dsp oversampling code.

## Goal
The ceiling is a hard guarantee, not a measured bound, and the owner can see gain reduction.

## Scope (in)
- Exact interval-endpoint coverage of inter-sample peaks (+1 sample latency, reported via latency) instead of the empirical +0.011 dB overshoot bound.
- 1 ms look-ahead option tested.
- Full SPEC-017 AC matrix: 44.1/48/96 kHz × all ceilings × input gains, measured with the 16× reference true-peak meter.
- CPU bench (`just bench`) against the SPEC-017 budget.
- Gain-reduction meter in the rack slot (telemetry via the existing VXTM path or a small per-slot meter atomic; no allocation on the audio thread).

## Tests
The SPEC-017 ACs, no-alloc `process`, offline = realtime ≤ 1e-6, §4.3 zipper on ceiling/gain drags.

## Out
Multiband/stereo linking, dither inside the limiter.

`just check` must pass.

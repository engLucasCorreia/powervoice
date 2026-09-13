# S3-05 — True-peak limiter module (Slice 3)

- **Tier:** Opus (one Opus review, blocking findings only — DSP + RT)
- **Depends on:** T-103 (rack), H-01 (rack fix, before merge)
- **Spec:** SPEC-017 — implement its **lean-slice subset** section (read it first); the rest is hardening. MEMORY A-008 + the ebur128 caveat (never use ebur128 as the normative TP meter).
- **Read first:** CLAUDE.md, MEMORY.md (D-022, A-008, T-103 learnings), specs/SPEC-017-true-peak-limiter.md, crates/testkit (add the 16× TP reference there), crates/modules.

## Goal
The last module in the voice-over chain guarantees the export never exceeds the true-peak ceiling (default −1.0 dBTP), transparently when not limiting.

## Scope (in)
- `vox-dsp`: 4× polyphase true-peak detector (128 taps) + parabolic refinement; look-ahead gain computer (hold over the window, linear-in-dB release, two moving averages).
- `vox-modules`: `TruePeakLimiter` (`org.powervoice.true-peak-limiter`) — input gain, ceiling, release, look-ahead (restart on change); latency = look-ahead + 16 samples; parameter events delayed internally by the latency; `Telemetry` gain reduction; built-in registration.
- `vox-testkit`: 16× true-peak reference measurement (used by the ACs; ±0.0014 dB in the spec's simulation).
- Tests: output TP ≤ ceiling + 0.10 dB (16× reference) on stress signals (fs/4 squares, inter-sample-peak signals, full-scale noise, transients); ≤ ceiling + 0.25 dB on ebur128; below-ceiling input is bit-identical after latency; 1 kHz pushed 6 dB into the limiter THD+N ≤ −80 dB; release timing ±2 %; offline = realtime; no allocation in `process`.

## Out (deferred to hardening)
Full SPEC-017 AC list, CPU budget, limiter UI beyond the generic parameter panel + GR meter readout.

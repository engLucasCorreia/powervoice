# S3-04 — Noise reduction module + noise-print profile (Slice 3, library part)

- **Tier:** Opus (one Opus review of DSP/RT, blocking findings only)
- **Depends on:** T-103 (rack), H-01 (merged)
- **Spec:** SPEC-014 — implement the **[S3]** ACs of its §9 lean subset that concern the module and the profile (DSP, parameters, blob format, capture analysis function, no-profile passthrough, latency); the capture command/UI is S3-06; everything tagged [H] is hardening. MEMORY A-009.
- **Read first:** CLAUDE.md, MEMORY.md (D-022, A-009, T-103 learnings: §4.3 harness, `allow_delayed_effect`, latency modules delay parameter effects, initial telemetry in `activate`/`reset`), specs/SPEC-014-noise-reduction.md, docs/references.md (Boll, Ephraim–Malah), crates/module-api (`NoiseProfile` extension, state blob), crates/modules (Gain pattern).

## Goal
A voice-over-grade spectral noise reducer that works in the rack from a captured noise print, measurably removing noise without hurting the voice.

## Scope (in)
- `vox-dsp`: STFT analysis/synthesis (Hann, 75 % overlap or as SPEC-014 says) with `realfft` (add the dependency to `vox-dsp` only — named by SPEC-014), decision-directed Wiener gain (β 0.98), frequency + time smoothing in dB, gain floor from Reduce by / Noise reduction %, Sensitivity; profile analysis (8192-point mean power density + log-power spread per bin) and conversion to other FFT sizes/rates.
- `vox-modules`: `NoiseReduction` (`org.powervoice.noise-reduction`) with SPEC-014's parameters/defaults; latency = FFT size N samples (42.7 ms at 48 kHz default); FFT-size change → restart (rack handles the latency-aware fade); `NoiseProfile` extension (capture from a buffer, store/load the versioned 32 824-byte blob in the state); no profile → bit-exact passthrough with latency kept; "Output noise only" = input − processed.
- Tests: SPEC-014 [S3] module ACs — tone bursts + white noise at −50 dBFS RMS, Reduce by 12 dB → noise floor (testkit) drops ≥ 10 dB while tone RMS changes < 0.5 dB; profile blob round-trip bit-exact; latency exact; no-profile passthrough; offline = realtime (same-machine tolerance per SPEC-014); §4.3 with the spec's steady-signal substitution; no allocation in `process`.

## Out
Capture command + rack panel wiring (S3-06), profile graph, musical-noise metric calibration, CPU bench, cross-rate conversion ACs tagged [H].

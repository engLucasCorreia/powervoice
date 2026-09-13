# S3-02 — Dynamics (compressor + limiter) and Noise Gate modules (Slice 3)

- **Tier:** Opus (one Opus review, blocking findings only — DSP + RT)
- **Depends on:** T-103 (rack), H-01 (rack fix, before merge)
- **Specs (essential subset only):** SPEC-016 §4 shared engine (detector 5 ms peak / 20 ms RMS, τ = 63.2 % in the dB gain domain, 20 ms parameter ramps, crossfaded enables), Compressor section (threshold, ratio, knee width, attack, release, makeup; Peak/RMS) and Limiter section (sample-peak threshold/ceiling, attack, release) of `org.powervoice.dynamics@1.0.0` — AutoGate and Expander sections present in the schema but may be implemented as "not yet available" (disabled) in this slice; SPEC-013 Noise Gate `org.powervoice.noise-gate@1.0.0` (threshold, hysteresis, attack, hold, release, range, sidechain HPF 24 dB/oct at 100 Hz on by default). Gain-reduction via the `Telemetry` extension. MEMORY A-007.
- **Read first:** CLAUDE.md, MEMORY.md (D-022, A-007, T-103 learnings: §4.3 harness, `allow_delayed_effect` rules, write initial telemetry in `activate`/`reset`), specs/SPEC-016-dynamics.md §4 + compressor/limiter ACs, specs/SPEC-013-noise-gate.md.

## Goal
The two workhorse voice-over processors — a compressor/limiter and a noise gate — work in the rack, sound clean and are measurably correct.

## Scope (in)
- `vox-dsp`: detector, gain computer (hard/soft knee), attack/release smoother, gate state machine (hysteresis + hold), sidechain HPF (Butterworth 24 dB/oct), linear parameter ramps.
- `vox-modules`: `Dynamics` (compressor + limiter sections live; AutoGate/Expander stubs disabled) and `NoiseGate` modules via the Module API, registered as built-ins; Telemetry GR readouts.
- Tests (ModuleTestHost + testkit): compressor static curve within ±0.5 dB on steady sines across −60…0 dBFS; attack/release time constants ±10 %; gate open/close thresholds ±0.5 dB on tone bursts, hold ±1 ms, range attenuation ±0.2 dB, sidechain HPF keeps a −20 dBFS 40 Hz rumble from opening a −40 dBFS gate; §4.3 zipper for continuous params; offline = realtime ≤ 1e-6; no allocation in `process`.

## Out (deferred to hardening)
AutoGate + Expander sections (T-407), look-ahead, TransferCurve extension + dynamics graph UI (T-410), exact ±2 % / ±0.1 dB tightened ACs of SPEC-016, CPU budget measurement.

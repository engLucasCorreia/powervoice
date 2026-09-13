# T-208 — Live output analyzer: post-rack tap, FFT worker, 1/24-octave bands, `VXSA`, bottom-dock UI

- **Milestone / wave:** M2 / W3
- **Tier:** Sonnet (+ Opus review for the engine part)
- **Depends on:** T-205 (UI shell), M1 engine (T-105, T-107)
- **Spec refs:** SPEC-007 (analyzer ACs: tap point = output meter point, FFT 8192 Blackman-Harris at 48 kHz scaled by rate, 1/24-octave bands, Fast/Medium/Slow 50/150/500 ms averaging, peak hold 2 s then 12 dB/s, response time, update at telemetry rate, bottom dock), ADR-003 Amendment 1 (`VXSA`) · **ADR refs:** ADR-002 (no FFT on the audio thread)

## Goal
A live spectrum of what the user hears, cheap enough to leave on, reusable by the M4 EQ graph.

## Scope
**In:** RT-safe tap in the output callback copying post-rack (+ dry monitor) samples into an SPSC ring; analyzer worker thread computing FFT → band levels → averaging → peak hold; `VXSA` frames on `analyzer_subscribe` at the telemetry rate; golden `VXSA` fixture + decoder; bottom-dock analyzer component (log frequency axis, dB scale, response selector, peak-hold toggle); exported TS module for reuse by the EQ graph (M4).

**Out:** EQ graph (T-409).

## Acceptance tests to write
- [ ] SPEC-007 analyzer ACs per its test plan (band level of a bin-centred tone, averaging time constants, peak-hold timing, response time; 0 allocations in the output callback with the tap active).

## Definition of Done
- [ ] Tests first, passing; `just check` green; Opus review of the engine part passed; report in CLAUDE.md format.

# T-107 — Monitoring: off / dry / through rack, drift-corrected input→output ring, latency readout

- **Milestone / wave:** M1 / W3
- **Tier:** Opus (+ Opus review)
- **Depends on:** T-105
- **Spec refs:** SPEC-002 §2.7, §4.4 (AC-9…AC-12) · **ADR refs:** ADR-002 §4, §6, §8

## Goal
The talent can hear themselves with a latency they can judge: three monitoring modes, no clicks, no
periodic drift glitches, and a latency readout that matches reality within 1 ms.

## Scope
**In:**
- Monitor ring from the input callback to the output callback; drift servo (fill-level PI controller driving `rubato::Async`, clamped ±1000 ppm), target fill F* = max input period + max output period + 1 ms.
- Modes Off / Dry (after the rack, unity) / Through rack (summed into the rack input with playback); audible only while armed or recording; ≤ 10 ms fades on mode/arm changes; monitoring never reaches the take.
- Monitor underrun → fade out/in + monitor-dropout counter (no marker).
- Latency readout computation (input latency + F* + rack latency when through-rack + output latency), recomputed within 1 s of any change; amber ≥ 20 ms / red ≥ 40 ms flags; first-time headphone hint flag.

**Out:** UI (T-109), sandboxed-plugin latency (M8).

## Crates / files
`crates/engine` (`monitor`, `drift`), `crates/dsp` (async resampler wrapper).

## Acceptance tests to write
- [ ] SPEC-002 AC-9…AC-12 per its test plan (AC-11 runs 10 simulated minutes at ±200 ppm and 48 k → 44.1 k).

## Definition of Done
- [ ] Tests first, passing; `just check` green; RT rules respected; report in CLAUDE.md format.

# S3-03 — Parametric EQ module (Slice 3, library part; the graph is S3-07)

- **Tier:** Opus (one Opus review of the DSP/RT part, blocking findings only)
- **Depends on:** T-103, H-01 (merged). The EQ graph UI moved to **S3-07** (after S3-01).
- **Spec:** SPEC-015 — implement its **lean-slice subset** section (read it first); the rest is hardening. MEMORY A-008.
- **Read first:** CLAUDE.md, MEMORY.md (D-022, A-008, T-103 learnings incl. §4.3 harness and `allow_delayed_effect`), specs/SPEC-015-parametric-eq.md, docs/references.md (RBJ cookbook, Butterworth Qs), crates/module-api (ParamInfo, ResponseCurve), crates/modules (Gain as the pattern).

## Goal
The owner shapes the voice with an Audition-style parametric EQ in the rack — HP/LP, two shelves, five peaks — with a draggable response graph.

## Scope (in)
- `vox-dsp`: RBJ biquads in f64 (coefficients + state), Butterworth cascades for HP/LP 6–48 dB/oct, parameter ramps (log freq/Q, dB gain) with per-sample coefficient recompute while ramping.
- `vox-modules`: `ParametricEq` (`org.powervoice.parametric-eq`) with the band roles/ranges/defaults of SPEC-015, `ResponseCurve` from the same coefficients, registered as a built-in.
- ~~Graph~~ → **S3-07** (not in this ticket): custom panel in the rack slot: log-frequency axis 20 Hz–20 kHz, ±12 dB (±24 option), curve from `ResponseCurve` via a `rack_response_curve(slot, points)` command (binary or JSON of ≤ 512 points — `VXRC` framing is hardening), draggable nodes (freq/gain) sending `set_param_plain`-style plain values (implement `param_set_plain(slot, id, value)` command), wheel = Q, band enable by double-click.
- Tests: response vs analytic ±0.1 dB at 1/12-octave points (44.1/48/96 kHz); HP −3 dB at fc ± 1 %; default EQ bit-exact; disabled band bit-exact; §4.3 for drags; offline = realtime ≤ 1e-6; no allocation in `process`.

## Out (deferred to hardening)
Live analyzer overlay (needs T-208 + VXSA reconciliation), slope crossfade polish, keyboard node navigation, expanded view, full AC list of SPEC-015.

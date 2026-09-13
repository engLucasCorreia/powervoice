# T-304 — Record at cursor (Insert/Overwrite), punch-in with pre/post-roll, latency offset & calibration

- **Milestone / wave:** M3 / W2
- **Tier:** Opus (+ Opus review)
- **Depends on:** T-301 (history, journal records), M1 (T-105/T-106/T-107 engine, recording, monitoring), M2 (T-206 selection)
- **Spec refs:** SPEC-022 (all ACs), SPEC-002 (recording pipeline, dropouts, monitoring), SPEC-003 (transport, heard position), SPEC-004 (undo, refusals while recording), SPEC-008 §4.2 (marker mapping for Insert) · **ADR refs:** ADR-004 Amendment 4, ADR-003 Amendment 2, ADR-002 §6/§8 · MEMORY A-006 and the SPEC-022 follow-ups

## Goal
Voice-over retakes work like Audition's Punch and Roll: re-record exactly the selected words with
pre-roll context, sample-accurately aligned after latency compensation, click-free, one undo step.

## Scope
**In:**
- Engine record modes New / Insert / Overwrite / Punch with the SPEC-022 dispatch rule; phase machine PreRoll → Recording → PostRoll → Finished/Cancelled with `record_phase` / `record_finished` events; auto-stop at the selection end; Stop semantics per phase; Record-while-playing stops first.
- Take window computation with the recording offset (automatic from timestamps + per-device-pair manual/calibrated offset), capture-writer look-back (~2 s), journal records `take_begin` (mode/window/rolls/offset), `take_window`, `take_cancel`.
- Document results: Insert (shift, markers shift), Overwrite (replace/extend, markers stay), Punch (replace exactly [start, end), 10 ms equal-power crossfades rendered inside the range, outside bit-identical, markers stay); one undo entry "Record"/"Punch-in".
- Original audio muted while recording (optional "Hear original"); monitoring per SPEC-002 modes in every phase.
- Device loss / dropout rules per phase (SPEC-022).
- Calibration wizard: 5 log sweeps played after the rack with monitoring forced off, cross-correlation in `vox-dsp` (realfft), ≥ 4/5 agreement, confidence rejection, Verify pass; offsets persisted per (host, input, output, rate) in settings.
- `FakeBackend` extensions: loopback source, perfectly timed talent source, hidden unreported latency.
- UI: record panel mode toggle, pre/post-roll fields, right-click menu on Record, calibration dialog, selection lock while recording; keymap Shift+R behaviour.

**Out:** loop/multi-take recording, on-the-fly punch while playing (deferred per SPEC-022).

## Acceptance tests to write
- [ ] SPEC-022 AC-1…AC-21 per its test plan (exact sample results on the fake backend; pooled crossfade level checks; calibration accuracy ±1 sample for 5–200 ms synthetic latency at −40 dB noise; rejection of silence/noise; undo exactness).

## Definition of Done
- [ ] Tests first, passing; `just check` green; Opus review passed; report in CLAUDE.md format.

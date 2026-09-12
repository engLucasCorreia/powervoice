# T-008 — M1 spec wave: SPEC-000/001/002/003/004/012

- **Milestone / wave:** M0 / W3
- **Tier:** Opus for SPEC-000/002/004/012; Sonnet for SPEC-001/003 (the orchestrator dispatches one agent per spec, in parallel)
- **Depends on:** T-002, T-003, T-005
- **PROMPT refs:** §2, §3.1, §3.3 (undo), §3.4, §3.6 (settings), §5 · **ADR refs:** ADR-001..005 · Template: `specs/_TEMPLATE.md`

## Goal
Approved-quality specs for everything M1 builds, with numeric, testable acceptance criteria. Specs describe **behavior**; ADRs describe **structure** — reference ADRs rather than restating them.

## Specs
| Spec | Title | Must cover |
|---|---|---|
| SPEC-000 | Architecture overview | one-page summary linking ADRs; glossary (document time vs device time, snapshot, chunk, slot, take) |
| SPEC-001 | Audio devices & I/O | host/device enumeration, input device + input channel selection, output device, sample-rate/buffer-size selection and fallback, hot-plug refresh (≤ 2 s), device-lost behavior (notice, stop transport, no crash, recovery on replug), persistence of choices in settings |
| SPEC-002 | Recording & monitoring | new-file recording (48 kHz/24-bit default, selectable), pre-record input meter + clip hold, crash-safe take (kill -9 recoverable), take → undoable edit, monitoring off/dry/through-rack incl. latency warnings and drift behavior, what happens on xrun during recording (count + marker? decide) |
| SPEC-003 | Transport & playback | play/stop/pause, loop selection, return to start, playhead follow, playhead extrapolation contract, start latency < 50 ms, device/document rate mismatch (resample), Audition-compatible transport shortcuts (Space, Shift+Space — verify bindings) |
| SPEC-004 | Document model & undo | snapshot semantics, what is undoable (edits, markers, bake), memory budget behavior, destructive edit stops playback / marker edits don't, recovery flow UX after crash, session-file cleanup |
| SPEC-012 | Module API & rack | rack behavior visible to users: slot add/remove/reorder, per-slot and whole-rack bypass (click-free, no time jump), parameter smoothing expectations (no zipper noise: step change produces no component > −90 dBFS above the steady-state spectrum — define measurement), latency reporting, generic parameter UI rules derived from schema, offline render equivalence (realtime vs offline output identical within −120 dB for deterministic modules) |

## Acceptance
- [ ] Each spec follows the template, has ≥ 5 numbered ACs with tolerances where numeric, and a test plan mapping ACs to unit/integration/manual tests.
- [ ] No contradictions with PROMPT §2 or ADRs; conflicts reported, not silently resolved.
- [ ] Items marked ⚠ in `docs/references.md` that a spec relies on are verified or explicitly deferred.

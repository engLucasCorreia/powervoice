# T-106 — Recording: arm, input meter, clip detection, capture pipeline, dropouts, disk rules, take commit

- **Milestone / wave:** M1 / W3
- **Tier:** Opus (+ Opus review)
- **Depends on:** T-105
- **Spec refs:** SPEC-002 §2.1–§2.6 (AC-1…AC-8, AC-13…AC-16), SPEC-004 AC-15 · **ADR refs:** ADR-002 §1–§2, §7–§8, ADR-004 §6–§7, §9 · MEMORY D-017, D-020

## Goal
New-file recording is lossless, crash-safe and honest about damage, exactly as SPEC-002 specifies.

## Scope
**In:**
- Arm (opens the input stream; never persisted), input meter values (peak max-hold, 300 ms RMS) into telemetry, clip detection (|x| ≥ 0.99990) with latch and per-take counting (runs < 10 ms apart merged).
- New Recording flow in the engine: document rate/save format from the command, capture 32f at the document rate, capture-writer resampling when the input can't run at the document rate.
- Capture ring (10 s) → capture-writer thread → `vox-project` take writer (drain ≥ every 50 ms) + chunk store; start/stop alignment to capture time (±1 sample).
- Dropout detection from capture timestamps (§4.3), silence fill (≤ 2 s) + "Dropout N ms" markers; > 2 s = input loss; unreliable timestamps → "length unknown" markers without fill.
- Capture-ring overflow → stop and keep; disk floor 512 MiB → stop and keep; remaining-time computation; low-disk warning data.
- Input device lost → stop and keep; **output-only loss → keep recording** (AC-16).
- Take → one undoable "Record" edit with its markers; audio edits/undo/redo refused while recording; Add marker allowed.
- Live recording peaks (`VXRP` frames, 64 spp) produced for T-108.

**Out:** monitoring (T-107), UI (T-109), record-at-cursor and punch-in (T-304).

## Crates / files
`crates/engine` (`input`, `capture`, `record`, `meter`, `dropout`), `crates/project` (take integration only).

## Acceptance tests to write
- [ ] SPEC-002 AC-1…AC-8 and AC-13…AC-16 per its test plan; AC-6 includes a real child-process `SIGKILL` test.
- [ ] SPEC-004 AC-15 engine part.

## Definition of Done
- [ ] Tests first, passing; `just check` green; RT rules respected; report in CLAUDE.md format, including the dropout-threshold calibration result on the owner's PipeWire machine (manual run note).

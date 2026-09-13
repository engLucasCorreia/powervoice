# H-05 — Post-merge review: recording path (S1-04 + H-07)

- **Tier:** Opus (review + fix blocking findings; no re-review loop)
- **Depends on:** S1-04 (recording end-to-end), H-07 (live take peaks), S2-01 (edits refused while recording).
- **Read first:** CLAUDE.md (RT rules), MEMORY.md (S1-04, H-07, T-101 take-durability learnings), specs/SPEC-002-recording.md, SPEC-004 recovery sections, crates/engine/src/{record.rs,input.rs,capture.rs,control.rs}, crates/project/src/{take.rs,session.rs}, src-tauri/src/recording.rs.

## Goal
Recording is trustworthy: no glitches from the input callback, no lost audio on stop/crash/device loss, bounded memory.

## Review checklist
- Input RT callback: no alloc/locks/syscalls/logging; FTZ/DAZ; ring overflow counted, never blocking.
- Capture writer: `TakeCapture` sync cadence, `ChunkStore::sync()` before journal references, live-peaks lock only on the writer thread (H-07), memory bounded for long takes.
- Take durability: kill -9 mid-take → recovery restores all synced audio (test with a child process or simulated crash), torn last part, header repair.
- Device loss mid-take and Stop races (Stop during finishing, double Stop).
- Rate handling: input rate vs document rate (resampling itself is H-06 — report gaps only).

## Deliverable
Fix every blocking finding with a regression test; list non-blocking findings in the report (they become hardening tickets). `just check` must pass.

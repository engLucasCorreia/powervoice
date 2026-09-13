# H-10 — Recording follow-ups (non-blocking findings of the H-05 review)

- **Tier:** Sonnet (no review loop; blocking findings only)
- **Depends on:** H-05 (merged).
- **Read first:** CLAUDE.md (RT rules), MEMORY.md (S1-04, H-07, H-05 notes; `/tmp` quota gotcha), specs/SPEC-002-recording.md §2.4/§2.5/§4.3, crates/engine/src/{input.rs,capture.rs,control.rs,record.rs}, src-tauri/src/{recording.rs,document.rs,nr_capture.rs}, ui/src/lib/record/*.

## Scope (in), in priority order
1. **Stuck document after a failed take commit:** if `commit_take` fails (e.g. `TakeNotInStore`) the take stays open and the document refuses edits, new recordings and close until restart. Discard or recover the take so the document is usable again, and tell the owner (notice) what was kept.
2. **Clip lamp reset** at take start (SPEC-002 AC-2).
3. **nr_capture test temp-dir leak:** `/tmp/powervoice-app-nr-capture-cancel-*` (65 MB each) are left behind — sweep like src-tauri `document.rs::tmp_dir`.
4. **Input dropout detection** (SPEC-002 §2.4/§4.3, AC-7): detect callback gaps, fill with silence to keep timing, add a dropout marker, and post a notice.
5. **Disk-space floor + remaining recording time** (SPEC-002 §2.5, AC-13): stop and keep the take before the disk is full; show remaining time in the record panel.
6. **Live-peaks memory cap:** decimate or cap the H-07 peak list for multi-hour takes (today ~5.4 MB/h at 48 kHz).

## Tests
One regression test per item (fake backend; hold `fake.spawn_driver(..)` in live-engine tests), RT input callback still no-alloc.

## Out
Capture resampling (H-06), punch-in (T-304).

`just check` must pass.

# S2-02 — Peak normalize favorites (Slice 2)

- **Tier:** Sonnet
- **Depends on:** S2-01 (selection + edit plumbing)
- **Specs (essential subset only):** SPEC-010 — one-click Normalize to −1 dB / −0.1 dB / −3 dB (sample peak) on the selection or the whole file; custom "Normalize…" dialog (dB); no-op + notice for silent input; gain in f64 applied through `ChunkWriter`; one undo entry "Normalize".
- **Read first:** CLAUDE.md, MEMORY.md (D-022, A-002 normalize decisions), `crates/project` ChunkWriter/Session edit API.

## Goal
The owner's favourite one-click normalize buttons work, exactly and undoably.

## Scope (in)
- Engine job: peak scan (brute force) over selection/whole file, gain = target − peak, write gained audio via `ChunkWriter` → `Session` edit (Identity marker mapping), playback stops first; progress for long files (simple event).
- UI: three toolbar favourite buttons + Favorites menu + Effects → Normalize… dialog; notice for silent input.
- Tests: testkit peak −1.00 ± 0.01 / −0.10 ± 0.01 / −3.00 ± 0.01 dBFS; outside-selection samples bit-identical; undo restores the hash.

## Out (deferred to hardening)
% targets, cancel, 60-min timing AC, pyramid-accelerated scan, rack notice, remembered last value.

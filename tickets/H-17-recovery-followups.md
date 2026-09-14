# H-17 — Recovery & storage follow-ups (T-301)

- **Tier:** Sonnet (no review loop; blocking findings only)
- **Depends on:** T-301 (recovery, budget, housekeeping).
- **Read first:** CLAUDE.md, MEMORY.md (T-301 notes, A-015; H-05 crash-test pattern — process-spawning tests live in their own test binary; `/tmp` quota gotcha), specs/SPEC-004 (AC-3, AC-11, §2.5), ADR-004 Amendment 3, crates/project/src/{recovery.rs,budget.rs,session.rs}, src-tauri/src/housekeeping.rs, ui/src/lib/recovery/*.

## Scope (in)
1. **Recovery dialog (A-015):** stays open after one Recover/Discard and lists the remaining sessions; Decide later closes it.
2. **Timing ACs on a real disk:** run T-301's ignored AC-3 (≤ 50 ms undo on 20 000 pieces) and AC-11 (recovery time) in `just test-big`; report the numbers. Only if AC-11 misses: propose delta-encoded checkpoints (ADR-004 amendment draft), don't implement.
3. **Compaction off the document lock:** copy the live chunks without holding the document lock (snapshot + generation swap under a short lock), so edits don't wait during compaction.
4. **Markers pressed during an interrupted take:** journal them (or keep them in the take sidecar) so recovery restores them.
5. **"Memory for audio" setting UI** (the backend already applies `memory_budget_mib` live) in a small Preferences dialog, plus auto-clearing the disk-full banner once space recovers.
6. **Test hygiene:** killed test runs leak `/tmp/vox-project-*` — sweep them like `src-tauri/src/test_util.rs`.

## Tests
One per item; AC-3/AC-11 numbers in the report.

## Out
Delta checkpoints implementation (only proposed if needed).

`just check` must pass.

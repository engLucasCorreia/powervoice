# H-15 — Sidecar follow-ups (T-306)

- **Tier:** Sonnet (no review loop; blocking findings only)
- **Depends on:** T-306 (sidecar, recent files, second-instance warning), H-12 (view state persistence) if merged.
- **Read first:** CLAUDE.md, MEMORY.md (T-306 notes; H-05 child-process crash-test pattern; `/tmp` quota gotcha — use `src-tauri/src/test_util.rs::tmp_dir`), specs/SPEC-018 (ACs 6, 13, 18 and the recent-files missing-file flow), crates/project/src/sidecar.rs, crates/rack (SlotModel), src-tauri/src/{document.rs,settings.rs}, ui/src/lib/document/{RecentFilesMenu.svelte,recentFiles.svelte.ts,ConfirmDialog.svelte}.

## Scope (in)
1. **Recent file missing:** the dedicated dialog (Locate… / Remove from list / Cancel) instead of the toast; Locate re-points the entry.
2. **Read-only folder matrix (AC-13):** saving next to audio in a read-only folder → sidecar-only save fails clearly, full save offers Save As; opening from read-only media works and never tries to write the sidecar until a save.
3. **Crash safety (AC-6):** SIGKILL during a sidecar write leaves either the old or the new sidecar valid (child-process test pattern from H-05), plus the `.bak` rule.
4. **Per-slot leniency:** a malformed rack slot becomes a placeholder slot (module missing/unreadable, state kept verbatim) instead of invalidating the whole sidecar — needs `SlotModel` to tolerate an unknown/malformed module entry.
5. **Perf (AC-18):** sidecar read/write and the fingerprint fast path within the spec's budget (bench or timed test in `just test-big` if slow); recent-files existence checks within 300 ms off the UI thread.

## Tests
One per item; SPEC-018's AC wording as test names.

## Out
FLAC/other save containers (T-201), marker `kind` in the core model (T-303).

`just check` must pass.

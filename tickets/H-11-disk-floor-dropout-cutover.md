# H-11 — Disk-space floor + remaining time; dropout cutover (H-10 leftovers)

- **Tier:** Sonnet (no review loop; blocking findings only)
- **Depends on:** H-10 (dropout detection, `RecordingResult.dropouts`), H-05 (stop reasons, `writer_failed` pattern).
- **Read first:** CLAUDE.md (RT rules), MEMORY.md (H-05, H-06, H-10 notes; decisions A-010/A-011), specs/SPEC-002-recording.md §2.4 (dropout table) and §2.5 (disk space, AC-13), SPEC-004 housekeeping, crates/engine/src/{input.rs,capture.rs,control.rs,record.rs}, crates/project/src/store/prealloc.rs (existing `libc` use), src-tauri/src/recording.rs, ui/src/lib/record/*.

## Scope (in)
1. **Free-space query (A-010):** `libc::statvfs` on unix (already a `cfg(unix)` dep of `vox-project`), `windows-sys` `GetDiskFreeSpaceExW` on Windows (already in the dependency tree via tauri/cpal — add it as a direct `cfg(windows)` dependency of the crate that needs it). Behind a small provider trait so tests inject values.
2. **Disk floor (SPEC-002 §2.5, AC-13):** a periodic (~1 s) check on the control thread; below the hard floor, stop gracefully with a new `StopReason::DiskFull` (keep the take, like WriteError, `InputShared::disk_full` mirroring `writer_failed`), notice with what was kept. Below the soft threshold (< 10 min remaining) at Record, confirm first. Disable SPEC-004 housekeeping while recording.
3. **Remaining recording time** in the record panel (free bytes ÷ bytes per second at the document format), updated ~1 Hz.
4. **Dropout cutover (A-011):** gaps longer than ~2 s follow SPEC-002 §2.4's second row — treated as device loss (stop, keep the take up to the gap), not filled with silence.
5. **Dropouts on the resampled capture path:** splice silence + marks there too (in input-rate samples before the resampler, or mapped to document samples).

## Tests
Injected free-space provider → soft confirm, hard-floor stop with `DiskFull` and the take kept bit-exact; remaining-time math; > 2 s gap → stop; resampled-path dropout marker position within one resampler chunk. RT input callback still no-alloc.

## Out
Notice action buttons ("Go to first dropout") — needs a notice-action mechanism.

`just check` must pass.

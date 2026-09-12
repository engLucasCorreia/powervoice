# T-101 — Document model: session store, chunks, snapshots, markers, take writer

- **Milestone / wave:** M1 / W1
- **Tier:** Opus (+ Opus review)
- **Depends on:** M0 (T-009 rename merged)
- **Spec refs:** SPEC-004 (M1 ACs: AC-1, AC-5, AC-12 GC part, AC-15 project side), SPEC-002 §2.2–§2.3, AC-6 (take-file recovery, project part), SPEC-000 glossary · **ADR refs:** ADR-004 (+ Amendment 1), ADR-001, ADR-002 · **PROMPT refs:** §2, §3.3, §4

## Goal
`vox-project` provides the disk-backed document the engine plays and records into: a session store of
immutable chunks, `Arc` snapshots with a piece table and markers, a crash-safe take writer, a minimal
journal and basic undo/redo — so M1 can record a take, undo/redo it and play it back.

## Scope
**In:**
- Session directory + advisory lock + `meta.json` (ADR-004 §1). The root path is passed in (no Tauri dependency).
- Chunk store: 65 536-sample chunks, 64 MiB preallocated segments, write-through `memmap2`, CRC32, commit protocol (ADR-004 §2).
- Per-chunk min/max peak pyramid computed at commit and appended to `peaks.<gen>.bin` (IPC delivery is M2).
- `DocSnapshot` / piece table / markers / `rev` / `audio_rev`, O(log n) position lookup, marker shifting helpers (ADR-004 §3).
- Snapshot sample reader: copy range `[pos, pos+n)` into a caller buffer through segment guards (used by the engine's reader thread; never by the audio thread).
- `ChunkWriter` (append, progress, cancel → nothing committed).
- Crash-safe take writer: 32-bit float mono WAV, header patched + `fdatasync` about every 1 s, writer API that lets T-106 drain ≥ every 50 ms, RIFF rollover past 4 GiB, and WAV-tail recovery (length from file size when the header is stale).
- Journal: append-only, CRC-checked records with `fdatasync` for take commits and marker ops; every edit record and undo entry carries the optional opaque `attachment: Option<Vec<u8>>` (ADR-004 Amendment 1).
- Undo/redo stacks of snapshots (push/pop, redo cleared by a new edit); refusal API for "not while recording" (`error.not_while_recording`).
- Memory-budget accounting + LRU segment eviction (SPEC-004 §2.4, AC-5).
- Session GC: delete cleanly closed sessions at close and at startup (SPEC-004 §2.8, AC-12 M1 part).

**Out:** journal replay / crash-recovery UI and budgets/compaction (T-301), user-file import/save (T-201/T-202), edit ops (T-302), sidecar (T-306).

## Crates / files
`crates/project` (new modules: `session`, `store`, `snapshot`, `reader`, `take`, `journal`, `history`, `gc`). Allowed new deps: `memmap2` 0.9, `crc32fast`, `libc` (unix preallocation only). No `hound` (ADR-001: codecs live in `io`) — the take writer writes RIFF itself.

## Acceptance tests to write
- [ ] SPEC-004 AC-1 (snapshot isolation under 50 concurrent commits, FNV-1a equal).
- [ ] SPEC-004 AC-5 (512 MiB budget, 60-min fixture, 200 random seeks: resident counter ≤ budget + 128 MiB) — store-level test; the fake-backend playback part is completed in T-105.
- [ ] SPEC-004 AC-12 M1 part (session dir gone ≤ 5 s after clean close; 20 cycles leave `sessions/` empty; interrupted deletion completed at next start; locked sessions untouched).
- [ ] SPEC-004 AC-15 project side (audio edit / undo / redo refused while a take is open).
- [ ] SPEC-002 AC-6 project part: a take WAV truncated at 1 000 random offsets with a stale header recovers `floor((size − header)/4)` samples, no panic.
- [ ] Take → one undo entry containing audio + markers; undo → empty; redo → identical hash and markers (SPEC-002 AC-5, project part).
- [ ] Chunk commit: CRC verified on read; preallocation makes disk-full an error at segment creation (fault-injected).

## Definition of Done
- [ ] Tests first, passing; `just check` green; no `unsafe` without `// SAFETY:`; report in CLAUDE.md format.

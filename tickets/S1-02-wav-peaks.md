# S1-02 — WAV read/write + document peaks query (Slice 1, libraries only)

- **Tier:** Sonnet (no Opus review unless something touches durability ordering)
- **Depends on:** T-101 (merged)
- **Specs (essential subset only):** SPEC-005 WAV open (16/24/32f; stereo → average downmix), WAV save (16/24/32f, TPDF dither when reducing bit depth, float > 1.0 clipped for integer targets), atomic save; ADR-003 `VXPK` layout; ADR-004 §5 peaks, Amendment 5 (use `Session::set_floor` for imports; `Session` owns the chunk-sync rule).
- **Read first:** CLAUDE.md, MEMORY.md (D-022; T-101 learnings), `crates/project` public API.

## Goal
Pure Rust building blocks that S1-03 wires into the app: import a WAV into a session as the undo floor,
save a snapshot to WAV atomically, and answer peak queries for any window of the document.

## Scope (in)
- `vox-io`: `read_wav(path) → (rate, channels, stream of f32 frames)` via `hound` (16/24/32-bit int, 32f), downmix to mono by averaging; `write_wav(path, rate, bits, samples)` with TPDF dither (seeded) for 16/24-bit and clipping for integer targets; atomic write helper (temp in the same dir → fsync → rename).
- `vox-project`: `import_wav(session, reader)` streaming through `ChunkWriter` → `Session::set_floor`; `save_snapshot_wav(snapshot, path, bits)` using `SnapshotReader` + `vox-io` writer; `peaks(snapshot, spp, start, count) → Vec<(min,max)>` as the conservative union of per-chunk pyramid buckets over the piece table (levels 64…65536 spp; raw samples when spp < 64), plus a `VXPK` encoder helper (bytes per ADR-003).
- Tests: 16/24/32f round-trip bit-exact when no conversion is needed; stereo L=+sine R=−sine → silence; 16-bit TPDF residual −96.3 ± 0.5 dBFS on a −20 dBFS sine (testkit); peaks never under-report vs brute force on random piece tables; atomic save leaves the old file intact on a simulated failure; `VXPK` golden bytes.

## Out (deferred to hardening)
MP3/M4A/Ogg/FLAC import and FLAC export (T-202/T-201), cue/adtl markers, downmix dialog, clip prompt, metadata notices, progress events, peaks PARTIAL during import.

## Definition of Done
`just check` green; report in CLAUDE.md format incl. the deferred list.

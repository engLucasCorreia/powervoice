# T-002 — ADRs A: architecture overview, threading/RT, IPC, document storage

- **Milestone / wave:** M0 / W1
- **Tier:** Opus
- **Depends on:** —
- **PROMPT refs:** §2, §3, §4, §5 · **References:** `docs/references.md`, `MEMORY.md` gotchas

## Goal
Four accepted-quality ADRs in `docs/adr/` that later tickets implement without re-deciding: ADR-001, ADR-002, ADR-003, ADR-004 (format: `docs/adr/README.md`). Docs only — no code.

## Decisions already made (record them, justify them, fill in the details)
**ADR-001 Architecture overview & crate graph**
- Crates: `module-api` (no deps on others) ← `dsp` ← `modules` ← `rack` ← `engine`; `io`, `project`, `testkit` (dev-dep), `cli`, `src-tauri` (thin), M8: `plugin-host`, `plugin-sandbox`. Draw the dependency graph (mermaid) and state the allowed directions.
- `rack` is a pure chain host with no cpal dependency, shared by the real-time engine, offline render, export, bake, ACX check and the CLI — one code path.

**ADR-002 Threading & real-time rules**
- Threads: Tauri main/UI thread; control thread (device polling ~1 s, command handling); cpal input callback; cpal output callback; reader/prefetch thread (streams the current document snapshot into an `rtrb` ring, ~200 ms prefetch); capture-writer thread (crash-safe WAV, header patched ~1 s); worker pool for peaks/spectrogram/analysis.
- The audio thread never touches the memory-mapped chunk store; old snapshots are dropped off the audio thread (reader thread or `basedrop`).
- cpal input and output are separate streams, possibly on separate clocks: monitoring uses an input→output ring with drift correction (define the approach: fill-level servo with fractional resampling or sample drop/insert with crossfade — pick one and justify).
- Device rate ≠ document rate → resample in the reader path (`rubato`).
- UI→audio: lock-free command queue; parameter events flow to modules as per-block event lists with sample offsets (modules own smoothing — see ADR-005).
- FTZ/DAZ on audio threads; `assert_no_alloc` in tests; xrun/device-lost reported via an RT-safe event ring to the control thread.
- Transport/latency: displayed playhead = rendered position − rack latency − output latency (cpal timestamps); selection and markers stay in document time.

**ADR-003 IPC data paths & shared types**
- Commands for requests; events for low-rate state; `tauri::ipc::Channel` for streamed binary (peaks, spectrogram tiles, live recording peaks, meters if efficient); `tauri::ipc::Response` for one-shot binary. Define binary layouts (header + little-endian payload) for peaks and tiles.
- Playhead: send `(sample_pos, monotonic_time_ns, rate, playing)` ~30 Hz; UI extrapolates per animation frame.
- Meters: ~30–60 Hz.
- Shared Rust→TS types: choose between `ts-rs` and `tauri-specta` — **check (web) that tauri-specta supports Tauri 2.11** before choosing; if unsure, choose `ts-rs`. Types generated into `ui/src/lib/ipc/bindings.ts`, generation run by `just` and checked in `just check` (fail if stale).

**ADR-004 Document storage, snapshots, undo, journal**
- 60 min mono 48 kHz = 172.8 M f32 samples ≈ 691 MB; a whole-file normalize would double it if held in RAM.
- Immutable chunks of 64–256 k samples (choose a size and justify) in an append-only memory-mapped session file (location: per-document session dir under the OS cache/data dir — choose). A document snapshot = piece table (ordered list of `(chunk_id, offset, len)`) in `Arc<DocSnapshot>`; markers live in the snapshot (undoable, shift with edits).
- Undo = stack of snapshots; memory budget bounds resident chunks, not undo depth; define eviction.
- Peak summaries cached per chunk id (only changed chunks recompute after edits).
- Edit journal (append-only, fsync policy) + chunk file ⇒ crash recovery; define recovery procedure and when session files are garbage-collected.
- Destructive edit during playback: playback stops (marker edits don't).
- Recording: the take is written to a crash-safe WAV and chunked; it becomes one undoable edit when recording stops.
- Save: render snapshot to the target WAV (atomic write: temp file + rename).

## Scope
**In:** the four ADR files, and an updated index row in `docs/adr/README.md` is **not** yours (orchestrator updates it). Each ADR: Context / Decision / Consequences / Alternatives / Open questions. Mermaid diagrams welcome.
**Out:** code, specs, Module API (T-003).

## Acceptance
- [ ] Each ADR is consistent with PROMPT §2 LOCKED decisions and with each other.
- [ ] Every "choose/define" item above has a concrete decision with a one-paragraph rationale.
- [ ] Open questions that need the owner are listed explicitly.
- [ ] `just check` is not required (docs-only), but don't break anything.

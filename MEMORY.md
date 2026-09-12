# MEMORY.md — Project memory (curated by the orchestrator only)

## Status
- **Current milestone:** M0 Foundations — W1 starting
- **Last checkpoint:** none
- **Next action:** M0 W1 running: T-001 (Haiku), T-002/T-003 (Opus, docs-only) — all in main tree, disjoint files. Then W2: T-006 after T-001; T-004 after T-001+T-002; T-005 after T-001+T-003.

## Environment
- Rust 1.98.1 stable via pacman `rustup` (+ rustfmt, clippy, x86_64-pc-windows-gnu target); `just` 1.58 via pacman; Node 26.7 / npm 11.19; git 2.55.
- Worktrees: the Agent tool's `isolation: "worktree"` fails in this session (session started before `git init`). Use manual worktrees: `git worktree add .claude/worktrees/T-NNN -b ticket/T-NNN main`; the agent works there and commits on its branch; the orchestrator squash-merges and removes the worktree.
- Package naming: `vox-<crate>` for libraries (dirs stay `crates/<crate>`), `voxedit-cli` (bin `voxedit-cli`), `voxedit-app` (src-tauri).

## Decisions log
| ID | Date | Decision | Where |
|---|---|---|---|
| D-001 | 2026-09-12 | Tauri 2 + Rust engine + Svelte 5 UI; single-file mono editor; non-destructive rack; destructive normalize favorites | PROMPT §2 |
| D-002 | 2026-09-12 | External plugins CLAP/VST3/LV2/VST2/JSFX in M8, out-of-process sandbox; generic param UI first, native editors M9 | PROMPT §2, §3.7 |
| D-003 | 2026-09-12 | Specs written just-in-time per milestone (W0), approved at previous checkpoint | PROMPT §5 |
| D-004 | 2026-09-12 | New crates `rack` (pure chain host shared by engine/export/bake/CLI), `testkit`, `cli`; rack core + document model moved to M1 | PROMPT §4, §7 |
| D-005 | 2026-09-12 | Document model = disk-backed chunk store + Arc snapshots (piece table) + edit journal; undo = snapshot stack | ADR-004 (pending) |
| D-006 | 2026-09-12 | Installable modules packaged as CLAP bundles (`clack-plugin`) | ADR-006 (pending) |
| D-007 | 2026-09-12 | LAME (LGPL-2.0+) **loaded at runtime via `libloading`** with our own FFI table (mp3lame-encoder rejected: static-only); MP3 export disabled if lib missing. ysfx (Apache-2.0 lib) only in sandbox (crash isolation). VST2 gated, via Carla | ADR-007 |
| D-008 | 2026-09-12 | Module API per ADR-005 (`activate`/`process`/`reset`/`deactivate`, event lists with offsets, host-owned bypass, typed extensions); supersedes PROMPT §3.7 wording `prepare`/`tail_samples()`; `module-api` deps: serde + thiserror | ADR-005 |
| D-009 | 2026-09-12 | Installable modules = CLAP plugins via `clack-plugin` 0.2 in a `module-clap` crate, `org.voxedit.module-info/1` extension, `.voxmod` zip packages, run in the sandbox | ADR-006 |
| D-010 | 2026-09-12 | Sandbox: one process per plugin instance; pipelined realtime path (+1 block latency, reported), blocking offline path; failure → bypass + notice (playback), abort (export/bake) | ADR-008 |

## Conventions
- Ticket commits: `T-NNN: <summary>` squash-merged by the orchestrator.
- Model tiers: Haiku 4.5 (bootstrap/templates/docs), Sonnet 5 (UI/IPC/IO/editing/harness), Opus 5 (architecture/RT/DSP/plugins/reviews).
- Max 4 parallel agents per wave.

## Gotchas / learnings
- `cpal` 0.18: no per-channel input selection (open full device, deinterleave), no hot-plug events (poll ~1 s off-thread), input/output are separate streams (monitoring needs drift-corrected ring). Enable `pipewire`/`jack` features on Linux; ALSA headers still required.
- `hound` has no `cue`/`LIST adtl` support → custom RIFF chunk code.
- `ebur128` needs feature `precision-true-peak` for 4× oversampled TP.
- Mono BS.1770: 1 kHz sine at −20 dBFS ≈ −23.0 LUFS (single channel, G = 1.0; stereo −23 dBFS/ch ≈ −23 LUFS).
- WebKitGTK WebGL can be slow (esp. NVIDIA); workaround `WEBKIT_DISABLE_DMABUF_RENDERER=1`. Owner GPU: AMD Phoenix (Mesa).
- Tauri on Wayland: global shortcuts unsupported (we only need in-window), pointer-coordinate quirks reported.
- ReaPlugs are VST2-only (2016), no Linux build.

- (from T-002) tauri-specta is still 2.0.0-rc.25 with exact `specta` pins → we use **ts-rs 12**; generate with `TS_RS_LARGE_INT=number` (u64 → number; positions < 2^53).
- (from T-002) Tauri sync commands run on the main thread → make command handlers `async`.
- (from T-002) cpal `StreamInstant`s are not comparable across streams → map each stream to the app clock inside its own callback.
- (from T-002) rubato 5: `Async` (runtime-adjustable ratio), `Fft` (fixed ratio), `Slip`; `process_into_buffer` doesn't allocate.
- (from T-002) Windows: mmap views and `WriteFile` aren't guaranteed coherent → write through the mapping; preallocate segments or a full disk means SIGBUS.

## Risks
| Risk | Mitigation |
|---|---|
| Windows/macOS untested (owner has Linux only) | `just check-cross`; platform-neutral code paths; flag at each checkpoint |
| WebKitGTK rendering performance | T-007 spike in M0; Canvas2D/CPU-tile fallback |
| LAME LGPL linking | runtime-loaded `libmp3lame` via `libloading`; owner confirmation before M6 |
| VST2 legal exposure | gated ticket T-811; prefer Carla bridge |
| ysfx JIT crashes | confined to `plugin-sandbox` (library is Apache-2.0) |
| AAC decoding patents | legal review before public distribution (symphonia AAC is opt-in feature) |

## Open questions for the owner (ask at M0 checkpoint)
- Project license (none chosen yet — private).
- Final product name (working name "VoxEdit").
- (ADR-004) Session disk budget exceeded (proposed max(8 GiB, 8× doc), keep ≥ 2 GiB free): auto-drop oldest undo steps with a notice, or warn only?
- (ADR-004) Recoverable sessions: keep until the user discards them (proposed), or auto-delete after N days?
- (ADR-004) Opening a file imports it into the session store (~1.33× file size on disk for 24-bit). Acceptable? Slow HDDs may miss the < 3 s open target.
- (ADR-007) Confirm LAME via runtime-loaded `libmp3lame` (bundled as a separate replaceable library on Windows/macOS/AppImage; system dependency on deb/Arch). Needed before M6.
- (ADR-007) Project license: MIT OR Apache-2.0 (recommended) vs GPL-3.0-or-later (GPL-2.0-only is incompatible with Apache-2.0 deps).
- (ADR-008) VST2 via Carla bridge (T-811): approve or reject.
- (ADR-006) Module id namespace `org.voxedit.*` + app identifier `app.voxedit.editor` become permanent at M3 (first sidecar) — decide together with the product name.
- (ADR-006) Allow installing unsigned native modules? (sandbox isolates crashes, not malice)
- (ADR-008) One extra block of latency per sandboxed plugin (~5 ms at 256 frames) when monitoring through the rack — acceptable?

## Orchestrator follow-ups
- ~~Reconcile ADR-005 ts-rs with ADR-003~~ — done by T-003 (no ts-rs in `module-api`).
- ADR-005 open question "are rack edits undoable?" vs ADR-004 journal (rack state currently outside undo history) — decide in SPEC-004/SPEC-012.
- Verify ADR-007's table covers rayon, memmap2 0.9, libc, assert_no_alloc 1.1.2, ts-rs 12.0.1 (crc32fast/rayon/basedrop added by T-003).
- M8: ADR-002 RT rules need a narrow documented exception — the sandbox proxy's non-blocking wake is a syscall on the audio thread.
- PROMPT §3.7 wording (`prepare`, `tail_samples()`, "documented per adapter" dual-mono) is superseded by ADR-005 (D-008) — mention at M0 checkpoint.
- T-007 must also verify: raw channel/`Response` payloads arrive as ArrayBuffer on WebKitGTK; 30 vs 60 Hz telemetry cost; cpal `playback` timestamps meaningful on PipeWire; opening devices at document rate doesn't disturb other apps.
- Consider a `just deps-check` recipe enforcing ADR-001 rule 4 (forbidden crate edges).

## Ticket learnings
_(appended after each merged ticket)_
- **T-001** (Haiku, 2026-09-12): workspace + 9 crate skeletons, lints, justfile, `.githooks/pre-commit` (fmt check; `core.hooksPath` set). `just check` UI steps auto-activate once `ui/package.json` exists. Parallel tickets that edit root `Cargo.toml`/`justfile` must run in worktrees from now on.

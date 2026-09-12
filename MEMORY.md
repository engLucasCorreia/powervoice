# MEMORY.md — Project memory (curated by the orchestrator only)

## Status
- **AUTONOMOUS MODE (D-021, 2026-09-12, owner going to sleep):** "keep the development without asking me anything anymore … not doing checkpoints anymore … until the end of the tickets." → Run every milestone back-to-back with no owner checkpoints and no questions. Owner-facing choices use the documented recommended default and are logged as "D-0xx (autonomous)". Still gate every ticket (`just check`, Opus review where required). **T-811 VST2 stays gated** (needs explicit legal sign-off; skip it and report at the end). M8 deferred choices use defaults: sandboxed-plugin monitoring latency = accept + show (SPEC-002 OD-1 A); unsigned native modules = allowed with a warning dialog.
- **Current milestone:** M0 done (checkpoint passed 2026-09-12, decisions D-011…D-020). M1 starting after T-009 (code rename) merges; M2 spec wave (T-200) running in parallel.
- **Last checkpoint:** none (M0 is the first)
- **Status line superseded:** the "Next action" line below is historical; after owner answers, apply decisions to specs (OD boxes), mark specs/ADRs approved, then write full M1 ticket files (T-101…T-110) and dispatch M1 W1.
- **Next action:** M0 W1 running: T-001 (Haiku), T-002/T-003 (Opus, docs-only) — all in main tree, disjoint files. Then W2: T-006 after T-001; T-004 after T-001+T-002; T-005 after T-001+T-003.

## Environment
- Rust 1.98.1 stable via pacman `rustup` (+ rustfmt, clippy, x86_64-pc-windows-gnu target); `just` 1.58 via pacman; Node 26.7 / npm 11.19; git 2.55.
- Worktrees: the Agent tool's `isolation: "worktree"` fails in this session (session started before `git init`). Use manual worktrees: `git worktree add .claude/worktrees/T-NNN -b ticket/T-NNN main`; the agent works there and commits on its branch; the orchestrator squash-merges and removes the worktree.
- Orchestrator: never `cd` into a worktree in Bash — the harness then switches the session's primary working directory into it. Use `git -C <path>` and absolute paths / `--manifest-path`. If it happens anyway, `cd` back to the main repo **before** `git worktree remove` of that worktree (removing the shell's cwd breaks the shell).
- Squash-merging a ticket branch that predates other merges usually conflicts only in `Cargo.lock`: take `--ours` (main) and let `just check` re-resolve, then `git add Cargo.lock`.
- Edition 2024 reserves `gen` — don't use it as an identifier (testkit uses `signal`).
- Package naming: `vox-<crate>` for libraries (dirs stay `crates/<crate>`), `powervoice-cli` (bin `powervoice-cli`), `powervoice-app` (src-tauri).

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
| D-009 | 2026-09-12 | Installable modules = CLAP plugins via `clack-plugin` 0.2 in a `module-clap` crate, `org.powervoice.module-info/1` extension, `.voxmod` zip packages, run in the sandbox | ADR-006 |
| D-010 | 2026-09-12 | Sandbox: one process per plugin instance; pipelined realtime path (+1 block latency, reported), blocking offline path; failure → bypass + notice (playback), abort (export/bake) | ADR-008 |

## Conventions
- Ticket commits: `T-NNN: <summary>` squash-merged by the orchestrator.
- Model tiers: Haiku 4.5 (bootstrap/templates/docs), Sonnet 5 (UI/IPC/IO/editing/harness), Opus 5 (architecture/RT/DSP/plugins/reviews).
- Max 4 parallel agents per wave.

## Rules (enforced in review)
- RT allocation checks go **only** through `vox_module_api::test_util::no_alloc` (raw `assert_no_alloc::assert_no_alloc` is clippy-disallowed): `assert_no_alloc` runs in warn mode workspace-wide via feature unification, so raw calls would only warn and pass.
- Types promising "no reallocation" (fixed-capacity lists/buffers) must implement `Clone` by hand preserving capacity — derived `Clone` on a `Vec` drops spare capacity.
- T-103 handoff: (1) rack currently drops module output events — needs an RT drain to the control thread; (2) chain edits may move unchanged module instances from the retiring chain (pointer moves) instead of cold-restarting all slots; crossfade needs one extra `max_block` buffer; (3) module registry, same-offset/id event coalescing, dual-mono shim.

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

## Owner decisions at the M0 checkpoint (2026-09-12)
| ID | Decision |
|---|---|
| D-011 | License **MIT OR Apache-2.0** (LICENSE-MIT, LICENSE-APACHE; metadata in T-104) |
| D-012 | **Renamed to PowerVoice** (formerly the working name VoxEdit); app id `app.<name>.editor`, module ids `org.<name>.*` unless the owner gives a domain |
| D-013 | MP3 export via runtime-loaded `libmp3lame` (`libloading`) approved |
| D-014 | Shortcuts: **Shift+Space = Play from start** (plays from selection start or 0); **Record = Shift+R (provisional)**; loop key in SPEC-019. PROMPT §3.6 "Shift+Space record" was wrong |
| D-015 | SPEC-004 OD-1 drop oldest undo steps with notice · OD-2 keep recovery data until discarded · OD-3 accept import cost (< 3 s open on SSD/NVMe only) · OD-4 rack edits not undoable except bake restores the rack (ADR-004 Amendment 1) |
| D-016 | `WEBKIT_DISABLE_DMABUF_RENDERER=1` **on by default** on Linux, opt-out `POWERVOICE_WEBKIT_DMABUF=1` (ADR-009 Amendment 1; T-104) |
| D-017 | Output device lost while recording → **recording continues** (SPEC-001 §2.4 exception, SPEC-002 AC-16) |
| D-018 | **Stop returns the playhead to the play-start position**; Pause keeps it; engine-initiated stops behave like Pause (SPEC-003 §2.1, AC-9) |
| D-019 | Whole-rack A/B is listening-only; exports always render the rack |
| D-020 | Record works with only an input device (SPEC-001 §2.3) |
Deferred to the M8 checkpoint: VST2 via Carla, unsigned native modules, sandboxed-plugin monitoring latency (SPEC-002 OD-1).

## Open questions for the owner (M0 items below were resolved — see the table above)
- Project license (none chosen yet — private).
- Final product name (working name "PowerVoice").
- (ADR-004) Session disk budget exceeded (proposed max(8 GiB, 8× doc), keep ≥ 2 GiB free): auto-drop oldest undo steps with a notice, or warn only?
- (ADR-004) Recoverable sessions: keep until the user discards them (proposed), or auto-delete after N days?
- (ADR-004) Opening a file imports it into the session store (~1.33× file size on disk for 24-bit). Acceptable? Slow HDDs may miss the < 3 s open target.
- (ADR-007) Confirm LAME via runtime-loaded `libmp3lame` (bundled as a separate replaceable library on Windows/macOS/AppImage; system dependency on deb/Arch). Needed before M6.
- (ADR-007) Project license: MIT OR Apache-2.0 (recommended) vs GPL-3.0-or-later (GPL-2.0-only is incompatible with Apache-2.0 deps).
- (ADR-008) VST2 via Carla bridge (T-811): approve or reject.
- (ADR-006) Module id namespace `org.powervoice.*` + app identifier `app.powervoice.editor` become permanent at M3 (first sidecar) — decide together with the product name.
- (ADR-006) Allow installing unsigned native modules? (sandbox isolates crashes, not malice)
- (SPEC-003) Audition default shortcuts: Space = play/stop and Home = return to start are verified; **Record (PROMPT says Shift+Space) and loop-toggle bindings are unverified** (one source says Shift+Space = "play from start"). Owner knows Audition — confirm Record/loop keys. Also: adopt Audition's "return playhead to start on stop" preference (Shift+X)?
- (ADR-009) Make `WEBKIT_DISABLE_DMABUF_RENDERER=1` the default for Linux (`just dev` and packaged launcher), with an opt-out? Also: run the ADR-009 §6 manual input checklist (`just spike`, focused window: Space, Shift+Space, Ctrl+Z, Ctrl+Shift+Z, drag coordinates).
- (ADR-008) One extra block of latency per sandboxed plugin (~5 ms at 256 frames) when monitoring through the rack — acceptable?

## Orchestrator follow-ups
- ~~Reconcile ADR-005 ts-rs with ADR-003~~ — done by T-003 (no ts-rs in `module-api`).
- ADR-005 open question "are rack edits undoable?" vs ADR-004 journal (rack state currently outside undo history) — decide in SPEC-004/SPEC-012.
- Verify ADR-007's table covers rayon, memmap2 0.9, libc, assert_no_alloc 1.1.2, ts-rs 12.0.1 (crc32fast/rayon/basedrop added by T-003).
- M8: ADR-002 RT rules need a narrow documented exception — the sandbox proxy's non-blocking wake is a syscall on the audio thread.
- PROMPT §3.7 wording (`prepare`, `tail_samples()`, "documented per adapter" dual-mono) is superseded by ADR-005 (D-008) — mention at M0 checkpoint.
- T-007 must also verify: raw channel/`Response` payloads arrive as ArrayBuffer on WebKitGTK; 30 vs 60 Hz telemetry cost; cpal `playback` timestamps meaningful on PipeWire; opening devices at document rate doesn't disturb other apps.
- **SPEC-017 (TP limiter, M4 W0):** `ebur128` true peak reads ~+0.10 dB high near fs/4 (−2.886 dBTP vs analytic −2.990). A "ceiling −1.0 → pass if ≤ −0.9 dBTP" criterion leaves no margin for meter error — characterise the meter (or use an independent higher-oversampling reference in testkit) and set tolerances accordingly.
- **SPEC-000 cross-doc notes (T-008):** resolved by orchestrator — (1) CLAUDE.md now says "return ring", not basedrop; (2) ADR-002 capture overflow = stop + finalize at last good sample (matches ADR-004 §7.4); (3) ADR-002 input opens only when armed/recording, not merely monitoring ≠ off; (4) ADR-004 crash bound = ≤ 250 ms process crash / ~1.5 s power loss; (6) step-derived decimals already implemented in T-005 review round. Open: (5) bake undo restoring the pre-bake rack needs an opaque rack-state attachment per undo entry → ADR-004 amendment only if the owner accepts SPEC-004 OD-4. Still to update in SPEC-000 once its author finishes: TELEMETRY_RATE = 60 Hz (ADR-009), ADR-009 no longer "pending", note 6 resolved. SPEC-001 §2.3 amended: Record available with input device only (SPEC-002 open Q2).
- **Spec consistency pass:** ✅ done by orchestrator for SPEC-001 (PipeWire default host on Linux, ALSA fallback) and SPEC-003 (Stop = Pause in v1 pending the owner's return-to-start decision; AC-4 now sample-exact loop concatenation + reset-at-seam). Still to do when the T-008 Opus half lands: check SPEC-002 treats the Record shortcut as unverified like SPEC-003, and cross-check SPEC-000/002/004/012 against SPEC-001/003 and the ADRs.
- Consider a `just deps-check` recipe enforcing ADR-001 rule 4 (forbidden crate edges).

## Ticket learnings
_(appended after each merged ticket)_
- **T-007** (Sonnet + orchestrator Opus review, 2026-09-12): ADR-009 — WebGL2 primary, Canvas2D fallback (feature-detect `webgl2`, fall back on `webglcontextlost`); IPC `Response`/`Channel` raw payloads arrive as `ArrayBuffer`, ~80–88 MB/s for 10 MB; **telemetry default 60 Hz** (free); `WEBKIT_DISABLE_DMABUF_RENDERER=1` roughly halves frame time on AMD/Mesa too (p50 17 → 10 ms, ~59 → ~100 fps; slightly higher worst-case spikes); Hyprland doesn't throttle rAF for visible-but-unfocused windows. Gotchas: Tauri commands taking `AppHandle<R>` must stay generic over `R: tauri::Runtime` to work inside `ipc_commands!`; automation must race rAF/IPC steps against `setTimeout` watchdogs (one unexplained >15 min hang seen once). `just spike` (+ `POWERVOICE_SPIKE_EXIT=1`) is the reusable harness.
- **T-005 review outcome** (1 fix round; 1 blocking = derived Clone dropping EventList capacity): `ModuleTestHost` immediate-effect check is now **opt-out** — built-in modules must declare deliberately delayed params (time constants, thresholds, neutral-section enables) via `ModuleTestHost::allow_delayed_effect(id)`; event-timing/flush/state checks are latency-aware and run in `Offline` mode; `display_decimals()` derives text precision from step (exact round-trip); `ProcessStatus` is `#[non_exhaustive]` (deviation from ADR-005 sketch); workspace `serde_json` has `float_roundtrip`; `clippy.toml` disallows raw `assert_no_alloc`.
- **T-005** (Opus, 2026-09-12): ADR-005 implemented in `vox-module-api` (+ `test-util`: `ModuleTestHost`, `TestGain`, `TestRng`, `install_test_allocator!()`) and `vox-rack` skeleton. Gotchas: `serde_json` needs feature `float_roundtrip` for bit-exact f64 (sidecar in `project` must enable it); `assert_no_alloc` workspace dep has no default features, module-api enables `warn_debug`+`warn_release` (never enable `disable_release` → compile_error); every test binary using `ModuleTestHost` must call `vox_module_api::install_test_allocator!();`; clippy wants `.is_multiple_of(n)` instead of `x % n == 0`. Orchestrator decisions: stepped params' display decimals derive from step (exact text round-trip); module registry → T-103; T-103 coalesces same-offset same-id param events in place (latest wins).
- **T-006** (Sonnet + Opus review, 2026-09-12; 1 fix round, 2 blocking findings): `vox-testkit` = generators (PCG32 verified vs reference vector, Kellet pink, sweeps, bursts, Tech 3341/3342 cases), measurements, golden helpers, WAV I/O; `powervoice-cli gen|analyze [--json]`, `gen-fixtures` bin (`just fixtures`, 60-min file streams at ~15 MB RSS). **Measurement conventions:** `-inf` = digital silence, `NaN` = non-finite input anywhere (never masquerades as silence), `None`/"n/a" = input shorter than the window (noise floor 500 ms, momentary 0.4 s, short-term 3 s); JSON uses `null` for −inf/NaN. Noise generator levels are **RMS dBFS**. Noise floor is per channel, worst reported. `hound::WavWriter` accepts `Cursor<&mut Vec<u8>>`. `ebur128` only exposes current M/S windows → poll every 100 ms for max values.
- **T-004** (Sonnet, 2026-09-12): Tauri 2.11.5 / @tauri-apps/api 2.11.1 / cli 2.11.4, Svelte 5.57, Vite 8.3, Vitest 5.0, TypeScript 6.0, jsdom 30. Gotchas: `cargo test -p X` runs with cwd = package dir → pass absolute paths (`{{justfile_directory()}}`) for `TS_RS_EXPORT_DIR`; `#[tauri::command]` emits a hidden macro → import command modules by glob for `generate_handler!`; Vitest needs `resolve.conditions: ["browser"]` for Svelte `mount()`; Tauri CLI run from `ui/` needs `TAURI_APP_PATH=../src-tauri`; plain `cargo test` writes ts-rs output to `src-tauri/bindings/` (gitignored). `check-types` diffs only ts-rs-generated files. Main tree needs `npm --prefix ui ci` after merges touching `ui/package-lock.json`.
- **T-001** (Haiku, 2026-09-12): workspace + 9 crate skeletons, lints, justfile, `.githooks/pre-commit` (fmt check; `core.hooksPath` set). `just check` UI steps auto-activate once `ui/package.json` exists. Parallel tickets that edit root `Cargo.toml`/`justfile` must run in worktrees from now on.

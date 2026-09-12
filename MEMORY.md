# MEMORY.md — Project memory (curated by the orchestrator only)

## Status
- **Current milestone:** M0 Foundations — W1 starting
- **Last checkpoint:** none
- **Next action:** dispatch M0 W1 (T-001 bootstrap, T-002 ADRs A, T-003 ADRs B)

## Decisions log
| ID | Date | Decision | Where |
|---|---|---|---|
| D-001 | 2026-09-12 | Tauri 2 + Rust engine + Svelte 5 UI; single-file mono editor; non-destructive rack; destructive normalize favorites | PROMPT §2 |
| D-002 | 2026-09-12 | External plugins CLAP/VST3/LV2/VST2/JSFX in M8, out-of-process sandbox; generic param UI first, native editors M9 | PROMPT §2, §3.7 |
| D-003 | 2026-09-12 | Specs written just-in-time per milestone (W0), approved at previous checkpoint | PROMPT §5 |
| D-004 | 2026-09-12 | New crates `rack` (pure chain host shared by engine/export/bake/CLI), `testkit`, `cli`; rack core + document model moved to M1 | PROMPT §4, §7 |
| D-005 | 2026-09-12 | Document model = disk-backed chunk store + Arc snapshots (piece table) + edit journal; undo = snapshot stack | ADR-004 (pending) |
| D-006 | 2026-09-12 | Installable modules packaged as CLAP bundles (`clack-plugin`) | ADR-006 (pending) |
| D-007 | 2026-09-12 | LAME linked dynamically (LGPL); ysfx (GPLv3) only in sandbox binary; VST2 gated on owner legal sign-off | ADR-007 (pending) |

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

## Risks
| Risk | Mitigation |
|---|---|
| Windows/macOS untested (owner has Linux only) | `just check-cross`; platform-neutral code paths; flag at each checkpoint |
| WebKitGTK rendering performance | T-007 spike in M0; Canvas2D/CPU-tile fallback |
| LAME LGPL linking | dynamic linking; owner confirmation before M6 |
| VST2 legal exposure | gated ticket T-811; prefer Carla bridge |
| ysfx GPLv3 | confined to `plugin-sandbox` binary |

## Open questions for the owner
- Project license (none chosen yet — private).
- Final product name (working name "VoxEdit").

## Ticket learnings
_(appended after each merged ticket)_

# BOARD — ticket status (orchestrator-owned)

Status: `todo` → `ready` (deps done) → `in-progress` → `review` → `done` · `blocked` · `gated` (needs owner decision).
Tiers: H = Haiku 4.5 · S = Sonnet 5 · O = Opus 5 · "+OR" = Opus review required.
Tickets for M1+ are stubs; full ticket files are written in each milestone's W0.

## M0 — Foundations
| ID | Title | Wave | Tier | Deps | Status |
|---|---|---|---|---|---|
| T-001 | Bootstrap: workspace, crate skeletons, lints, justfile, hooks | W1 | H | — | done |
| T-002 | ADRs A: architecture, threading/RT, IPC, document storage | W1 | O | — | done |
| T-003 | ADRs B: Module API, module ABI (CLAP), licensing, sandbox outline | W1 | O | — | done |
| T-004 | Tauri 2 + Svelte 5 scaffold, theme, i18n, shared types, mocks | W2 | S | T-001, T-002 | in-progress |
| T-005 | `module-api` v0 + `rack` skeleton + module test host | W2 | O | T-001, T-003 | in-progress |
| T-006 | `testkit` + `voxedit-cli` gen/analyze + `just fixtures` | W2 | S+OR | T-001 | in-progress |
| T-007 | Platform spike: renderers, Wayland input, IPC throughput → ADR-009 | W3 | S+OR | T-004 | todo |
| T-008 | M1 spec wave: SPEC-000/001/002/003/004/012 | W3 | O/S | T-002, T-003, T-005 | todo |

## M1 — Core engine, recording & playback
| ID | Title | Wave | Tier | Deps | Status |
|---|---|---|---|---|---|
| T-101 | Document model: chunk store, snapshots, piece table, markers, take writer | W1 | O | M0 | todo |
| T-102 | Devices: enumeration, channel deinterleave, hot-plug polling, device lost | W1 | O | M0 | todo |
| T-103 | Rack core: chain, host bypass/crossfade, param routing, latency sum, Gain, offline render, `cli render --rack` | W1 | O | M0 | todo |
| T-104 | App infra: tracing log, RT event ring, typed errors→toasts, panic hook, settings, stores, keymap registry | W1 | S | M0 | todo |
| T-105 | Backend trait + cpal/fake backends, RT thread, command queue, transport, reader prefetch, SR resampling | W2 | O | T-101, T-102, T-103 | todo |
| T-106 | Recording: crash-safe WAV, peaks stream, take → undoable edit | W3 | O | T-105 | todo |
| T-107 | Monitoring off/dry/through-rack, drift-corrected in→out ring | W3 | O | T-105 | todo |
| T-108 | Meters + Tauri commands/events + playhead (pos, time) | W3 | S | T-104, T-105 | todo |
| T-110 | Bench harness (divan, callback-time histogram) | W3 | S | T-105 | todo |
| T-109 | UI: device settings, transport bar, meter bridge, live recording waveform | W4 | S | T-106, T-107, T-108 | todo |

## M2 — Editor view
| ID | Title | Wave | Tier | Deps | Status |
|---|---|---|---|---|---|
| T-201 | WAV 16/24/32f r/w + cue/adtl chunks | W1 | S | M1 | todo |
| T-202 | Import (symphonia) + stereo downmix + FLAC encode | W1 | S | M1 | todo |
| T-203 | Per-chunk peak pyramid + binary IPC | W1 | S+OR | M1 | todo |
| T-204 | STFT spectrogram tile service | W1 | O | M1 | todo |
| T-205 | Waveform renderer: zoom/scroll, amplitude zoom, rulers, extrapolated playhead | W2 | S | T-203 | todo |
| T-206 | Selection model, time formats, zero-crossing snap | W3 | S | T-205 | todo |
| T-207 | Spectrogram renderer, log/linear, split view | W3 | S | T-204, T-205 | todo |
| T-208 | Live output spectrum analyzer | W3 | S | T-205 | todo |

## M3 — Editing & history
| ID | Title | Wave | Tier | Deps | Status |
|---|---|---|---|---|---|
| T-301 | Undo/redo, resident memory budget, edit journal, crash recovery | W1 | O | M2 | todo |
| T-302 | Cut/copy/paste/delete, trim, silence, insert silence | W2 | S | T-301 | todo |
| T-303 | Markers panel + navigation | W2 | S | T-301 | todo |
| T-304 | Record at cursor (insert/overwrite) + punch-in + latency calibration | W2 | O | T-301 | todo |
| T-305 | Peak normalize favorites | W2 | S+OR | T-301 | todo |
| T-306 | Sidecar `.vo.json`, autosave, recent files | W3 | S | T-302, T-303 | todo |

## M4 — Effects modules & rack UI
| ID | Title | Wave | Tier | Deps | Status |
|---|---|---|---|---|---|
| T-401 | Latency compensation, latency_changed re-activation, A/B dry delay | W1 | O | M3 | todo |
| T-402 | Parametric EQ DSP + ResponseCurve | W1 | O | M3 | todo |
| T-403 | Dynamics A: detector, compressor, limiter | W1 | O | M3 | todo |
| T-404 | True-peak limiter | W1 | O | M3 | todo |
| T-405 | Rack panel + generic parameter UI | W2 | S | T-401 | todo |
| T-406 | Module & rack presets | W2 | S | T-401 | todo |
| T-407 | Dynamics B: expander + AutoGate | W2 | O | T-403 | todo |
| T-408 | Noise gate (shared envelope/hysteresis) | W3 | O | T-407 | todo |
| T-409 | EQ graph UI (analyzer + ResponseCurve) | W3 | S | T-402, T-405, T-208 | todo |
| T-410 | Dynamics UI + gain-reduction meter (Telemetry) | W3 | S | T-405, T-407 | todo |

## M5 — Noise reduction
| ID | Title | Wave | Tier | Deps | Status |
|---|---|---|---|---|---|
| T-501 | Offline NR algorithm (decision-directed Wiener + smoothing) + goldens | W1 | O | M4 | todo |
| T-502 | Noise print capture + profile storage (state blob) | W1 | S | M4 | todo |
| T-503 | Streaming NR module, latency_changed on FFT size | W2 | O | T-501, T-502 | todo |
| T-504 | NR UI: capture, profile graph, output-noise-only | W3 | S | T-503 | todo |

## M6 — Loudness & export
| ID | Title | Wave | Tier | Deps | Status |
|---|---|---|---|---|---|
| T-601 | Loudness analysis I/S/M/LRA/TP | W1 | S+OR | M5 | todo |
| T-602 | Bake rack (offline render, pre-roll, latency trim, tail) | W1 | O | M5 | todo |
| T-603 | LUFS normalize favorites | W2 | S | T-601 | todo |
| T-604 | ACX check on processed output | W2 | S | T-601, T-602 | todo |
| T-605 | Encoders (LAME dynamic), rubato, TPDF dither | W2 | S+OR | T-602 | todo |
| T-606 | Export dialog | W3 | S | T-605 | todo |

## M7 — Polish & packaging
| ID | Title | Wave | Tier | Deps | Status |
|---|---|---|---|---|---|
| T-701 | Shortcut map audit (Audition-compatible) | W1 | S | M6 | todo |
| T-702 | i18n audit | W1 | H | M6 | todo |
| T-703 | Settings audit | W1 | S | M6 | todo |
| T-704 | Performance tuning vs targets | W1 | O | M6 | todo |
| T-705 | AppImage/deb, Win/mac build docs, README/user guide | W2 | S | T-701..T-704 | todo |

## M8 — External plugins
| ID | Title | Wave | Tier | Deps | Status |
|---|---|---|---|---|---|
| T-801 | Shared-memory ring + wakeup shim + crash/hang/gain test plugins | W1 | O | M7 | todo |
| T-802 | Sandbox process, watchdog, proxy module | W2 | O | T-801 | todo |
| T-803 | CLAP adapter (+ enumerate) | W3 | O | T-802 | todo |
| T-804 | Scanner orchestration, cache, blocklist | W3 | S | T-802 | todo |
| T-805 | Our modules as CLAP packages (clack-plugin) | W4 | O | T-803 | todo |
| T-806 | VST3 adapter (coupler vst3 + moduleinfo scan) | W4 | O | T-803, T-804 | todo |
| T-807 | LV2 adapter (livi) | W4 | O | T-803, T-804 | todo |
| T-808 | JSFX adapter (ysfx, sandbox-only) | W4 | O | T-803, T-804 | todo |
| T-809 | Plugin manager UI + "Install module…" | W5 | S | T-804 | todo |
| T-810 | Plugin state in presets/sidecar | W5 | S | T-803 | todo |
| T-811 | VST2 adapter (prefer Carla bridge) | W5 | O | T-803 + owner legal sign-off | gated |

## M9 — Native plugin editors
| ID | Title | Wave | Tier | Deps | Status |
|---|---|---|---|---|---|
| T-901 | Plugin GUI as floating window owned by sandbox (HWND/NSView/X11-XWayland) | W1 | O | M8 | todo |

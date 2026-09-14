# BOARD — ticket status (orchestrator-owned)

Status: `todo` → `ready` (deps done) → `in-progress` → `review` → `done` · `blocked` · `gated` (needs owner decision).
Tiers: H = Haiku 4.5 · S = Sonnet 5 · O = Opus 5 · "+OR" = Opus review required.
Tickets for M1+ are stubs; full ticket files are written in each milestone's W0.

## ▶ Vertical slices — CURRENT PLAN (owner D-022, 2026-09-13)
Build thin end-to-end slices that work, then harden. Milestone sections further down are the **hardening backlog**: their full-spec scope is done after the slices (tickets partly absorbed by a slice are marked "→ S#").

### Slice 1 — Record & play (open WAV, see waveform, play, record, save)
| ID | Title | Tier | Deps | Status |
|---|---|---|---|---|
| S1-01 | Playback end-to-end: engine core, output device, rack in path, transport, VXTM telemetry, transport + device UI | O | T-101, T-102, T-103 | done |
| S1-02 | WAV read/write (`vox-io`) + document peaks query (`vox-project`) — libraries only | S | T-101 | done |
| S1-03 | Open/Save WAV in the app + Canvas2D waveform view (zoom/scroll/playhead/click-to-seek) | S | S1-01, S1-02 | done |
| S1-04 | Recording end-to-end: input device, arm, input meter, record → take → document, live waveform | O | S1-01 | done |

### Slice 2 — Edit
| ID | Title | Tier | Deps | Status |
|---|---|---|---|---|
| S2-01 | Selection + cut/copy/paste/delete/trim/silence + undo/redo end-to-end | S | S1-01, S1-03 | done |
| S2-02 | Peak normalize favorites (−1/−0.1/−3 dB + custom) end-to-end | S | S2-01 | done |
| S2-03 | Markers basic: add (M), panel, rename, delete, jump, WAV cue round-trip | S | S1-03, S2-01 | done |

### Slice 3 — Clean & shape
| ID | Title | Tier | Deps | Status |
|---|---|---|---|---|
| S3-01 | Rack panel + generic parameter UI end-to-end | S | S1-01, H-01 | done |
| S3-02 | Dynamics (compressor + limiter) + Noise Gate modules | O | T-103, H-01 | done |
| S3-03 | Parametric EQ module (library part, lean SPEC-015) | O | T-103, H-01 | done |
| S3-07 | EQ graph panel (ResponseCurve, draggable nodes) | S | S3-01, S3-03 | done |
| S3-04 | Noise reduction module + noise-print profile (library part, lean SPEC-014) | O | T-103, H-01 | done |
| S3-06 | Capture Noise Print command + NR slot panel | S | S3-01, S3-04, S2-01 | done |
| S3-05 | True-peak limiter (lean SPEC-017) | O | T-103 | done |

### Slice 4 — Deliver
| ID | Title | Tier | Deps | Status |
|---|---|---|---|---|
| S4-01 | Loudness analysis + LUFS normalize | S | S2-02, S1-01 | done |
| S4-02 | Export encoders (FLAC, LAME via libloading, offline resampler) — library part | S | S1-01, S1-02 | done |
| S4-04 | Export job + dialog (ACX preset) | S | S1-03, S4-02 | done |
| S4-03 | ACX check report | S | S4-01 | done |

### Hardening & backlog
| ID | Title | Tier | Status |
|---|---|---|---|
| H-01 | Rack: bypass toggle during a new instance's warm-up hold re-introduces the cold-output click (T-103 re-review #1) + restart of load-failed slots, stuck held-back restart, count dropped moved events | O | done |
| H-02 | Save: stream `save_snapshot_wav` (today holds the whole document in RAM, ~691 MB for 60 min); move TPDF dither from `vox-io` into `vox-dsp::dither` (ADR-001 §4) | S | done |
| H-03 | TP limiter: exact interval-endpoint coverage (+1 sample latency) instead of the measured +0.011 dB bound; test 1 ms look-ahead; full SPEC-017 AC matrix (44.1/96 kHz, all ceilings/gains), CPU bench, GR meter UI | O | done |
| H-04 | Playback engine fixes from the S1-01 post-merge review (fade state machine, host switch, rack-latency drain) | O | done |
| H-05 | Post-merge Opus review of S1-04 (RT input callback, capture writer, take durability) — deferred for session budget | O | done |
| H-06 | UX: New Recording dialog (rate/bit depth); capture-writer resampling (Save-in-prompt → Save As for untitled recordings: done in the window-close hotfix) | S | done |
| H-07 | Live waveform while recording (writer-thread peaks → `record_peaks_get` → 10 Hz UI poll); owner-reported S1-04 gap | S | done |
| H-08 | Export follow-ups from S4-04: Whole file / Selection choice (range already plumbed end to end); export reads the current rack model once S3-01 lands (today renders an empty rack); SPEC-014 "Output noise only" confirmation; MP3 VBR in the dialog | S | done |
| H-10 | Recording follow-ups from the H-05 review: document stuck after a failed take commit; input dropout detection + markers; disk-space floor / remaining time; clip lamp reset at take start; live-peaks memory cap (nr_capture test temp-dir leak: fixed on main); H-06 gaps: 88.2 kHz, Record on a non-empty document opens the dialog, kHz formatting | S | done |
| H-11 | Disk-space floor + remaining recording time (A-010: libc statvfs / windows-sys); dropouts > 2 s → device-loss stop (A-011); dropout marks on the resampled path | S | done |
| H-12 | Spectral view follow-ups (T-207): persist display prefs in Settings (A-014), one shared time ruler + scrollbar in EditorView (SPEC-007 §2.1 stacking), HiDPI columns, import-freeze message once import has progress state | S | todo |
| H-13 | WebGL2 renderers (ADR-009 primary): spectrogram R8 textures + colormap shader, waveform; Canvas2D stays the fallback; FFT-size texture gating live | S+OR | todo |
| H-14 | FLAC export is malformed: flacenc frame numbering makes `flac -t` warn (not seekable) and symphonia reject our own exports — fix or replace the encoder; then FLAC verify-before-rename (SPEC-005 §2.11) | S | done |
| H-09 | Normalize follow-ups from S2-02: progress via the S4-04 `job_progress` event + cancel for long files; % targets; Effects menu entry; pyramid-accelerated peak scan (SPEC-010 AC-9/AC-15); remembered dialog value | S | done |

## M0 — Foundations
| ID | Title | Wave | Tier | Deps | Status |
|---|---|---|---|---|---|
| T-001 | Bootstrap: workspace, crate skeletons, lints, justfile, hooks | W1 | H | — | done |
| T-002 | ADRs A: architecture, threading/RT, IPC, document storage | W1 | O | — | done |
| T-003 | ADRs B: Module API, module ABI (CLAP), licensing, sandbox outline | W1 | O | — | done |
| T-004 | Tauri 2 + Svelte 5 scaffold, theme, i18n, shared types, mocks | W2 | S | T-001, T-002 | done |
| T-005 | `module-api` v0 + `rack` skeleton + module test host | W2 | O+OR | T-001, T-003 | done |
| T-006 | `testkit` + `powervoice-cli` gen/analyze + `just fixtures` | W2 | S+OR | T-001 | done |
| T-007 | Platform spike: renderers, Wayland input, IPC throughput → ADR-009 | W3 | S+OR | T-004 | done |
| T-008 | M1 spec wave: SPEC-000/001/002/003/004/012 | W3 | O/S | T-002, T-003, T-005 | done |
| T-009 | Rename code/config to PowerVoice + license metadata | post-checkpoint | S | M0 | done |

## M1 — Core engine, recording & playback
| ID | Title | Wave | Tier | Deps | Status |
|---|---|---|---|---|---|
| T-101 | Document model: chunk store, snapshots, piece table, markers, take writer | W1 | O | M0 | done |
| T-102 | Devices: enumeration, channel deinterleave, hot-plug polling, device lost | W1 | O | M0 | done |
| T-103 | Rack core: module registry, chain swap/retire, host bypass/crossfade, dual-mono shim, param routing + same-offset/id event coalescing, latency sum, Gain, offline render, `cli render --rack` | W1 | O | M0 | done |
| T-104 | App infra: tracing log, typed errors→toasts/banners, panic hook, settings, stores, keymap registry, WebKit DMA-BUF default | W1 | S | M0 | done |
| T-105 | Engine core: control thread, RT callbacks, transport (Stop→play start), reader/prefetch, resampling, heard clock, 60 Hz telemetry | W2 | O | T-101, T-102, T-103 | → S1-01 (subset), rest hardening |
| T-106 | Recording: crash-safe WAV, peaks stream, take → undoable edit | W3 | O | T-105 | → S1-04 (subset), rest hardening |
| T-107 | Monitoring off/dry/through-rack, drift-corrected in→out ring | W3 | O | T-105 | done |
| T-108 | IPC layer: M1 commands/events, VXTM/VXRP channels, binary decoders + golden fixtures, clock sync | W3 | S | T-104, T-105 | → S1-01/S1-03 (subset), rest hardening |
| T-110 | Bench harness (divan, callback-time histogram) | W3 | S | T-105 | todo |
| T-109 | UI: device settings, transport bar, meter bridge, live recording waveform | W4 | S | T-106, T-107, T-108 | → S1-01/S1-04 (subset), rest hardening |

## M2 — Editor view
| ID | Title | Wave | Tier | Deps | Status |
|---|---|---|---|---|---|
| T-200 | M2 spec wave: SPEC-005 formats, SPEC-006 waveform view, SPEC-007 spectral & analyzer | W0 | O/S | M0 | done |
| T-201 | Save pipeline: WAV writer 16/24/32f, TPDF dither, clip policy, cue/adtl markers, atomic save job, FLAC encode | W1 | S+OR | M1 | → S1-02/S1-03 (WAV subset), rest hardening |
| T-202 | Import pipeline: symphonia decode (WAV variants, FLAC, MP3, M4A, Ogg), downmix, session import job, CLI convert/markers | W1 | S+OR | M1 | done (import job UI + channel-choice dialog → T-209) |
| T-203 | Per-chunk peak pyramid + binary IPC | W1 | S+OR | M1 | → S1-02/S1-03 (subset), rest hardening |
| T-204 | STFT spectrogram tile service | W1 | O | M1 | done |
| T-205 | Waveform renderer: zoom/scroll, amplitude zoom, rulers, extrapolated playhead | W2 | S | T-203 | → S1-03 (Canvas2D subset), rest hardening |
| T-206 | Selection model, time formats, zero-crossing snap | W3 | S | T-205 | todo |
| T-207 | Spectrogram renderer, log/linear, split view | W3 | S | T-204, T-205 | done |
| T-208 | Live output analyzer: post-rack tap, FFT worker, 1/24-oct bands, VXSA, bottom-dock UI | W3 | S+OR | T-205 | in progress |
| T-209 | File menu & dialogs: Open, Save, Save As, downmix dialog, save format, clip prompt, unsaved-changes prompt, import progress | W3 | S | T-201, T-202, T-205 | → S1-03 (subset), rest hardening |

## M3 — Editing & history
| ID | Title | Wave | Tier | Deps | Status |
|---|---|---|---|---|---|
| T-300 | M3 spec wave: SPEC-008, SPEC-009, SPEC-010, SPEC-018, SPEC-022 | W0 | O | M0, SPEC-005 | done |
| T-301 | Undo/redo, resident memory budget, edit journal, crash recovery | W1 | O | M2 | todo |
| T-302 | Cut/copy/paste/delete, trim, silence, insert silence, clipboard | W2 | S+OR | T-301 | todo |
| T-303 | Markers: kinds, add/region, rename, drag, delete, navigation, Markers panel | W2 | S | T-301 | todo |
| T-304 | Record at cursor (Insert/Overwrite) + punch-in + latency offset & calibration | W2 | O | T-301 | todo |
| T-305 | Peak normalize favorites | W2 | S+OR | T-301 | todo |
| T-306 | Sidecar `.vo.json`, identity check, sidecar-only saves, recent files, second-instance warning | W3 | S+OR | T-302, T-303 | in progress |

## M4 — Effects modules & rack UI
| ID | Title | Wave | Tier | Deps | Status |
|---|---|---|---|---|---|
| T-400 | M4 spec wave: SPEC-015 parametric EQ + SPEC-017 true-peak limiter (part 1); SPEC-013 noise gate + SPEC-016 dynamics (part 2) | W0 | O | M0 | done |
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
| T-500 | M5 spec wave: SPEC-014 noise reduction (module, noise print capture, UI) | W0 | O | M0 | done |
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

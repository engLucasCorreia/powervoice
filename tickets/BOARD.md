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
| H-12 | Editor view follow-ups (T-207, T-306): app-wide default display prefs in Settings (per-document ones already in the sidecar), one shared time ruler + scrollbar in EditorView (SPEC-007 §2.1), waveform zoom/scroll/selection persisted in the sidecar view, HiDPI columns | S | done |
| H-16 | Analyzer follow-ups (T-208): floor/ceiling, frequency zoom, hover, device state, View menu toggle, settings persistence; wire `telemetry_rate_hz` (30 Hz) into VXTM/VXMT/VXSA | S | done |
| H-19 | Real menu bar (File/Edit/View/Effects/Help dropdowns, keyboard access, shortcuts shown) replacing the flat always-visible "menu" toolbars | S | done |
| H-20 | Save/open gaps (SPEC-005): dither None, keep FLAC when opened FLAC + full WAV promotion table, format-mapped/metadata-dropped notices, stereo→mono save warning, progressive import display | S | done (live partial peaks during import need a peaks_progress channel — deferred) |
| H-22 | Preset UX (T-406): rename in slot/rack menus, overwrite-confirm on save, manage dialog, import/export preset files | S | done |
| H-21 | Punch-in follow-ups (T-304): markers during an operation, live take drawn at the punch point, device loss per phase, talent-source exactness tests (drift, dropouts), AC-11/AC-15, Settings → Recording page | S | done (polish → H-23) |
| H-23 | Punch-in polish (A-016): no phantom PostRoll on output loss, waveform follows the record head during an operation, AC-13 length-unknown variant | S | done |
| H-24 | OWNER-REPORTED TOP PRIORITY: resizable layout (splitters, persisted), analyzer growth fix, correct scales/grids/units (analyzer Hz/dB, spectral ruler, waveform dBFS + adaptive time ruler, EQ axes), no overlaps | S | done (percent amplitude mode + tunable caps → follow-up) |
| H-25 | OWNER REQUEST: big-tech-quality visual design — design audit + design system + tokens + component kit (phase 1), applied across the app after H-24 (phase 2); ui-ux-designer agent | O | done (Windows button order, shared popover → follow-up) |
| H-26 | Design follow-ups (H-25): Windows dialog button order (primary first), shared menu/popover component for all menus, migrate `lucide-svelte` (deprecated) → `@lucide/svelte`, Unicode minus in analyzer dB labels, analyzer "dBFS" unit label overlapping the top tick (−20), owner-verified screens with a file open and while recording | S | done |
| H-27 | Playback playhead follow (SPEC-003 §2.1/§3, SPEC-006 §2.8 continuous band-follow; spec'd but never implemented — found by H-23), reconciled with H-23's record-head page-flip | S | done |
| H-28 | Post-H-26/H-27 fixes: `?preview&scene=recording` `effect_update_depth_exceeded` in WaveformView follow effects, transport store null guard when telemetry precedes `transport_get`, View-menu toggle for `Settings.playhead_follow`, Normalize target fields in true minus | S | done |
| H-29 | Plugin follow-ups (T-809): uninstall action (remove installed file + registry, confirm, documents keep a Missing slot), deterministic duplicate-id policy across install/rescan, move plugin fixtures into `ui/src/lib/test/fixtures.ts` | S | done |
| H-30 | Job-service consistency (T-602): export and save hold `SpectroService::begin_background_job()` (SPEC-007 lists them), every job service emits its error notice before the terminal `Failed` event (audit export/loudness/normalize/nr_capture/calibration), a rack edit during a bake is refused instead of discarded at commit | S | done |
| H-31 | Theme follow-ups (T-708): Match System also honours `prefers-contrast: more` → High Contrast (A-019), High Contrast column in the component gallery, WebGL thick sample lines in HC (quad-based), preview IPC transport command cases (H-32/T-709) | S | done |
| H-32 | EQ graph draws grid but no curve/labels at 2126×850 (all themes; first-layout measuring bug) — found in T-708 screenshots | S | done |
| H-33 | Small fixes (H-22): `sanitize_preset_name` must never yield a leading `.` (hidden by `list_json_names` as an atomic temp file), de-flake `WelcomeOffer.test.ts` (full-suite-only failure) | S | done |
| H-34 | Plugin polish (T-806): de-flake `sandbox/tests/teardown.rs::nothing_outlives_its_owner` (fd baseline after warm-up release), Linux Install Module wording for picking a file inside a `.vst3` bundle, rename scan cache to `plugin-scan.json` with migration | S | done |
| H-35 | Zoom commands (T-701 found SPEC-006 §2.6 claims them but they don't exist): Zoom to Selection, Zoom Full, vertical amplitude zoom (Alt+= / Alt+-) with Audition bindings, View menu + toolbar entries | S | done |
| H-36 | Fix flaky LV2 test `crates/sandbox/tests/lv2.rs::a_latency_change_goes_through_the_worker_and_asks_for_a_restart` (offline worker round trip fails 8/10 in isolation on main) | S | done |
| H-37 | Loop playback (SPEC-003 AC-4; engine has none — found by T-401): loop region = selection, seamless wrap, playhead history across the wrap (ADR-002 §8), Loop transport button + shortcut | O | done |
| H-38 | Packaging (T-807, A-022): deb Recommends `liblilv-0-0`, Arch optdepends `lilv`, AppImage note; user-guide note that LV2 needs lilv | H | done |
| H-39 | Loop in the menus (H-37): Loop Playback checkbox item (View or a Transport section, shared Menu + Ctrl/⌘+L chip), tour step mention | H | done |
| H-40 | Live recovery of Missing plugin slots (T-810): when an install or rescan registers a module that an open document's Missing slot references, re-resolve that slot in place with its kept state blob (no reopen needed), with a notice | S | done |
| H-41 | Output level meter (owner req.): vertical, fixed-size (no analyzer resize), standard peak/RMS ballistics + peak hold, readable throttled numerals, clip latch — keep the lively jump | S | in progress |
| H-42 | Analyzer diagnostics (owner req.): peak frequency labels (Hz + note), voice statistics panel (F0, sibilance → de-esser suggestion, mud/presence, hum, rumble, noise floor), LTAS over selection/document, compare/freeze snapshots, optional Spectrum Inspector window, "add EQ band here" | O | in progress |
| H-43 | Idle CPU (owner report): web view main thread ~1 core while idle — draw-on-demand frame scheduler instead of perpetual rAF loops, idle telemetry throttling, meters/analyzer stop at rest; target ≤ 2 % idle (release) with a regression guard | O | todo (after H-41, H-42, T-704) |
| H-44 | `.voxmod` locales + presets (T-805, ADR-006 §7 step 5): merge a package's `locales/` into i18n at load and index its `presets/` as factory presets for that module; remove both on uninstall | S | todo |
| H-45 | Packaging: bundle `powervoice-sandbox` in local `just build` (Tauri externalBin + build step; the release workflow already passes it via `--config`), verify plugins load from the AppImage/.deb, CI job for `just check` on pushes | S | todo |
| H-18 | Shared test factories for DTOs (`ui/src/lib/test/fixtures.ts` + Rust builders) so a new DTO field touches one helper, not 20 test literals (merge-friction fix) | H | done |
| H-17 | Recovery follow-ups (T-301): dialog stays open for remaining sessions (A-015), AC-3/AC-11 timing on a real disk (test-big), compaction off the document lock, markers during an interrupted take, Memory-for-audio UI, `/tmp/vox-project-*` sweep | S | done |
| H-15 | Sidecar follow-ups (T-306): recent-file-missing dialog, read-only folder matrix (AC-13), SIGKILL-during-sidecar-save test (AC-6), per-slot leniency for a malformed rack slot, perf budget (AC-18) | S | done |
| H-13 | WebGL2 renderers (ADR-009 primary): spectrogram R8 textures + colormap shader, waveform; Canvas2D stays the fallback; FFT-size texture gating live | S+OR | done (renderer setting UI → H-19; owner smoke on real hardware pending) |
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
| T-110 | Bench harness (divan, callback-time histogram) | W3 | S | T-105 | done |
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
| T-206 | Selection model, time formats, zero-crossing snap | W3 | S | T-205 | done |
| T-207 | Spectrogram renderer, log/linear, split view | W3 | S | T-204, T-205 | done |
| T-208 | Live output analyzer: post-rack tap, FFT worker, 1/24-oct bands, VXSA, bottom-dock UI | W3 | S+OR | T-205 | done (follow-ups → H-16) |
| T-209 | File menu & dialogs: Open, Save, Save As, downmix dialog, save format, clip prompt, unsaved-changes prompt, import progress | W3 | S | T-201, T-202, T-205 | done (gaps → H-20) |

## M3 — Editing & history
| ID | Title | Wave | Tier | Deps | Status |
|---|---|---|---|---|---|
| T-300 | M3 spec wave: SPEC-008, SPEC-009, SPEC-010, SPEC-018, SPEC-022 | W0 | O | M0, SPEC-005 | done |
| T-301 | Undo/redo, resident memory budget, edit journal, crash recovery | W1 | O | M2 | done (follow-ups → H-17) |
| T-302 | Cut/copy/paste/delete, trim, silence, insert silence, clipboard | W2 | S+OR | T-301 | → S2-01 (done); rest: insert silence, cross-document clipboard |
| T-303 | Markers: kinds, add/region, rename, drag, delete, navigation, Markers panel | W2 | S | T-301 | → S2-03 (done); rest: marker kind in the core model, drag, region→selection |
| T-304 | Record at cursor (Insert/Overwrite) + punch-in + latency offset & calibration | W2 | O | T-301 | done (follow-ups → H-21) |
| T-305 | Peak normalize favorites | W2 | S+OR | T-301 | done (via S2-02 + H-09) |
| T-306 | Sidecar `.vo.json`, identity check, sidecar-only saves, recent files, second-instance warning | W3 | S+OR | T-302, T-303 | done (follow-ups → H-12, H-15, T-303) |

## M4 — Effects modules & rack UI
| ID | Title | Wave | Tier | Deps | Status |
|---|---|---|---|---|---|
| T-400 | M4 spec wave: SPEC-015 parametric EQ + SPEC-017 true-peak limiter (part 1); SPEC-013 noise gate + SPEC-016 dynamics (part 2) | W0 | O | M0 | done |
| T-401 | Latency compensation, latency_changed re-activation, A/B dry delay | W1 | O | M3 | done |
| T-402 | Parametric EQ DSP + ResponseCurve | W1 | O | M3 | done (via S3-03) |
| T-403 | Dynamics A: detector, compressor, limiter | W1 | O | M3 | done (via S3-02) |
| T-404 | True-peak limiter | W1 | O | M3 | done (via S3-05 + H-03) |
| T-405 | Rack panel + generic parameter UI | W2 | S | T-401 | done (via S3-01) |
| T-406 | Module & rack presets | W2 | S | T-401 | done (rename/overwrite UX → H-22) |
| T-407 | Dynamics B: expander + AutoGate | W2 | O | T-403 | → S3-02 (params implemented); rest: verify SPEC-016 part-2 ACs |
| T-408 | Noise gate (shared envelope/hysteresis) | W3 | O | T-407 | done (via S3-02) |
| T-409 | EQ graph UI (analyzer + ResponseCurve) | W3 | S | T-402, T-405, T-208 | → S3-07 (done); rest: analyzer overlay, keyboard nodes, expanded view |
| T-410 | Dynamics UI + gain-reduction meter (Telemetry) | W3 | S | T-405, T-407 | → H-03 (GR meters done); rest: VXTC transfer curve + dynamics graph UI |

## M5 — Noise reduction
| ID | Title | Wave | Tier | Deps | Status |
|---|---|---|---|---|---|
| T-500 | M5 spec wave: SPEC-014 noise reduction (module, noise print capture, UI) | W0 | O | M0 | done |
| T-501 | Offline NR algorithm (decision-directed Wiener + smoothing) + goldens | W1 | O | M4 | done (via S3-04) |
| T-502 | Noise print capture + profile storage (state blob) | W1 | S | M4 | done (via S3-04 + S3-06) |
| T-503 | Streaming NR module, latency_changed on FFT size | W2 | O | T-501, T-502 | done (via S3-04) |
| T-504 | NR UI: capture, profile graph, output-noise-only | W3 | S | T-503 | → S3-06 + H-08 (done); rest: profile graph, Clear Noise Print, Ctrl+Shift+P |

## M6 — Loudness & export
| ID | Title | Wave | Tier | Deps | Status |
|---|---|---|---|---|---|
| T-601 | Loudness analysis I/S/M/LRA/TP | W1 | S+OR | M5 | done (via S4-01) |
| T-602 | Bake rack (offline render, pre-roll, latency trim, tail) | W1 | O | M5 | done |
| T-603 | LUFS normalize favorites | W2 | S | T-601 | done (via S4-01 + H-09) |
| T-604 | ACX check on processed output | W2 | S | T-601, T-602 | done (via S4-03) |
| T-605 | Encoders (LAME dynamic), rubato, TPDF dither | W2 | S+OR | T-602 | done (via S4-02 + H-02 + H-14) |
| T-606 | Export dialog | W3 | S | T-605 | done (via S4-04 + H-08) |

## M7 — Polish & packaging
| ID | Title | Wave | Tier | Deps | Status |
|---|---|---|---|---|---|
| T-701 | Shortcut map audit (Audition-compatible) | W1 | S | M6 | done |
| T-702 | i18n audit | W1 | H | M6 | done |
| T-703 | Settings audit | W1 | S | M6 | done |
| T-704 | Performance tuning vs targets | W1 | O | M6 | in progress |
| T-705 | AppImage/deb, Win/mac build docs, README/user guide | W2 | S | T-701..T-704 | done (Win/mac installers documented, not built) |
| T-706 | Complete architecture & developer docs (owner req.): docs hub, C4/crate/threading/sequence Mermaid diagrams, data/IPC/DSP/plugins/UI docs, contributing guide, ADR index, docs check script | W3 | O | T-809 | todo |
| T-707 | Docs for everyone (owner req.): what-is / how-it-works with Mermaid, complete task-based user guide, FAQ, glossary with plain-language analogies, friendly README | W4 | S | T-706, T-708, T-709 | todo |
| T-708 | Themes (owner req.): Light ("clear") mode at full parity incl. canvas/WebGL renderers from tokens, Match System, High Contrast, View → Theme menu, no-flash, 4-theme screenshots + raw-colour lint | W2 | O | T-809, H-28 | done |
| T-709 | Guided Tour widget (owner req.): spotlight + anchored step cards, Welcome tour + contextual tours, first-run offer, Help → Take the Tour, `Settings.tours` | W3 | O | T-708 | done |

## M8 — External plugins
| ID | Title | Wave | Tier | Deps | Status |
|---|---|---|---|---|---|
| T-801 | Shared-memory ring + wakeup shim + crash/hang/gain test plugins | W1 | O | M7 | done |
| T-802 | Sandbox process, watchdog, proxy module | W2 | O | T-801 | done |
| T-803 | CLAP adapter (+ enumerate) | W3 | O | T-802 | done |
| T-804 | Scanner orchestration, cache, blocklist | W3 | S | T-802 | done |
| T-805 | Our modules as CLAP packages (clack-plugin) | W4 | O | T-803 | done |
| T-806 | VST3 adapter (coupler vst3 + moduleinfo scan) | W4 | O | T-803, T-804 | done |
| T-807 | LV2 adapter (livi) | W4 | O | T-803, T-804 | done |
| T-808 | JSFX adapter (ysfx, sandbox-only) | W4 | O | T-803, T-804 | done |
| T-809 | Plugin manager UI + "Install module…" | W5 | S | T-804 | done |
| T-810 | Plugin state in presets/sidecar | W5 | S | T-803 | done |
| T-811 | VST2 adapter (prefer Carla bridge) | W5 | O | T-803 + owner legal sign-off | gated |

## M9 — Native plugin editors
| ID | Title | Wave | Tier | Deps | Status |
|---|---|---|---|---|---|
| T-901 | Plugin GUI as floating window owned by sandbox (HWND/NSView/X11-XWayland) | W1 | O | M8 | in progress |

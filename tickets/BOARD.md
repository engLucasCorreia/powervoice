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
| H-41 | Output level meter (owner req.): vertical, fixed-size (no analyzer resize), standard peak/RMS ballistics + peak hold, readable throttled numerals, clip latch — keep the lively jump | S | done |
| H-42 | Analyzer diagnostics (owner req.): peak frequency labels (Hz + note), voice statistics panel (F0, sibilance → de-esser suggestion, mud/presence, hum, rumble, noise floor), LTAS over selection/document, compare/freeze snapshots, optional Spectrum Inspector window, "add EQ band here" | O | done |
| H-43 | Idle CPU (owner report): web view main thread ~1 core while idle — draw-on-demand frame scheduler instead of perpetual rAF loops, idle telemetry throttling, meters/analyzer stop at rest; target ≤ 2 % idle (release) with a regression guard | O | done |
| H-44 | `.voxmod` locales + presets (T-805, ADR-006 §7 step 5): merge a package's `locales/` into i18n at load and index its `presets/` as factory presets for that module; remove both on uninstall | S | done |
| H-45 | Packaging: bundle `powervoice-sandbox` in local `just build` (Tauri externalBin + build step; the release workflow already passes it via `--config`), verify plugins load from the AppImage/.deb, CI job for `just check` on pushes | S | done |
| H-46 | Rack pre-roll on Play/seek (T-704, A-026): playback start with the voice rack is 49.4 ms because rack latency (NR 42.7 ms + limiter) is added to every start — pre-roll the rack from the prefetch so audio starts < 50 ms regardless of rack latency; SPEC-003/SPEC-012 amendment | O | done |
| H-47 | Split-view frame spikes (T-704): WebGL2 waveform+spectral p99 ~120 ms on tile arrival (texture upload/tile scheduling budget per frame), Canvas2D spectral fallback < 60 fps, re-check output-callback worst case on an idle machine — after H-43 | O | done |
| H-48 | UI polish from screenshots: toolbar selection readout (Start/End/Length) clips its values at 2126 px, output meter scale labels crowd at short dock heights, meter bar thin vs its column; consolidate H-42 peak-marker decay onto H-41's `PeakBallistics` | S | done |
| H-49 | Packaging (T-901): deb Recommends `libsuil-0-0` (LV2 plugin windows) and libX11 where needed; user-guide note on plugin windows (Linux X11/XWayland, Windows untested, macOS not yet) | H | done |
| H-50 | Test-infra flakes: `normalize::tests::start_peak_job_runs_end_to_end_and_reports_done_with_a_result` races the Result event after Progress(Done) (H-30's notice-before-Failed rule doesn't cover success), `vox-sandbox-ipc::crash_is_bypassed_and_reported` under load; re-check the output-callback worst case (T-704 open question) on an idle machine | S | done |
| H-51 | Doc/code gaps found by T-706: ADR-001 §2/§4 crate graph and job ownership, ADR-002 §1 (no rayon pool, 60 Hz telemetry, 65 536 monitor ring), ADR-004 §1 sessions use `data_dir()` (roaming on Windows) while modules use `app_local_data_dir` — a real Windows risk, SPEC-013/016 hidden-inert params (AutoGate/Expander/look-ahead, `TransferCurve` missing), output meter excludes the dry monitor signal but the analyzer tap includes it (comment claims identical), stale comments in loudness.rs/document_commands.rs | S | done |
| H-52 | Bench hygiene (H-47): find and silence the `Tried to (de)allocate memory in a thread that forbids allocator calls!` printed once per `just bench-callback` row (setup/teardown outside the counted callback — reads like an RT violation), confirm the waveform-pane p95 on a quiet machine, then run a full `just bench && just test-big && just bench-ui && just perf-matrix` so docs/performance.md is regenerated consistently | S | done |
| H-53 | Calibration success-path test (H-50): no automated test covers a successful loopback calibration — port the private `Rig`/`Loopback` rig from `crates/engine/tests/punch.rs` into shared test support so `src-tauri/src/calibration.rs` can assert Result-before-Done end to end | S | done |
| H-54 | The last failing performance row: `frame_spectral_canvas2d_2126x850_p95_ms` 21.2 ms vs SPEC-007 AC-10's ≤ 16.7 ms — the Canvas2D (software) split view at a large window during a continuous zoom+scroll sweep. Either optimise further (the WebGL2 path passes everywhere) or amend AC-10 to scope the 60 fps target per renderer/window size, with the owner's 2126×850 window in mind | S | done |
| H-55 | Packaged-module determinism failure on CI: `voxmod.rs::the_module_test_host_suite_passes_through_the_adapter` — two instances of the packaged Gain differ at sample 10558 on GitHub's 2-core runner, passes locally (5/5, incl. single-threaded and under load); CI run 35064469629 | O | done |
| H-70 | Save as a real job (H-60, SPEC-005 §4.10): `JobKind::Save` with progress/cancel and `error.save.in_progress`, plus the §2.7 step-1 pre-write free-space estimate using H-11's `FreeSpaceProvider` | S | done |
| H-71 | Progressive peaks during import (H-60/H-20, SPEC-005 §2.3 + SPEC-006 AC-13): VXPK's PARTIAL bit and the decoder exist but nothing emits them — stream partial peaks so a long import draws progressively instead of appearing at the end | S | done |
| H-72 | Small spec gaps (H-60): amplitude ruler percent mode (SPEC-006 §2.4) and the `notice.open.markers_unreadable`/`markers_out_of_range` notices for malformed WAV cue chunks (behaviour is already safe, just silent) | H | done |
| H-73 | Shared spectral test helper (H-60): every crate hand-rolls a DFT for tone/harmonic assertions — put one BH4/FFT analysis helper in `vox-testkit` and adopt it in the existing call sites | S | done |
| H-74 | Small notice/fixture follow-ups (H-67): suppress the dropout notice's "Go to first" when SPEC-022 §2.12 filtered every dropout out of the committed window (have `commit_take_op` report whether it placed a marker), and add a `noticeFixture` to `ui/src/lib/test/fixtures.ts` so the next Notice field doesn't touch 10 literal sites | H | done |
| H-75 | Save concurrency follow-ups (H-70): let the write run without holding the document lock (SPEC-005 §2.7 says editing continues) — needs `History::mark_saved_at(seq)` first or the dirty flag mis-clears for edits made during the write; and make the pre-flight overs scan cancellable (SPEC-005 §2.8) | S | done |
| H-76 | Import-view follow-up (H-71): SPEC-005 §2.3 item 4 — zoom, scroll and selection should work while an import runs (today the importing waveform draws but the view's own interactions and the previous document's overlays are left alone) | H | done |
| H-77 | Dynamics panel remainder (H-63 hand-off, T-410): binary `VXTC` frame + golden fixture and the AC-23 timing test, the custom Dynamics panel (global latency readout, section panels, gain-reduction meters, AutoGate lamp — AC-20/AC-22), the operating-point dot (needs gr_total + effective makeup, so panel-local), always-on handle labels and per-component curve overlays | O | done |
| H-78 | Refresh the docs for everything since T-706/T-707 (meters, analyzer, editing, markers, effects, saving, import, platforms, architecture) | S | done |
| H-79 | Owner-reported: the selection wash is the same blue as the waveform and is painted over it — give the selection its own colour and keep the waveform readable inside it, in all four themes | S | done |
| H-80 | Owner-requested: loop the whole document when nothing is selected (selection looping already works; SPEC-003 §2.2's "inert with no selection" rule is revised) | S | done |
| H-81 | Owner-reported bug: turning Loop off keeps looping the old region and the pink loop strip never disappears | S | done |
| H-82 | Editing during an import: SPEC-005 §2.3 item 4 says it is disabled, but nothing gates it (H-76's open question — a UX gap, not a data bug) | S | done |
| H-83 | Svelte ownership warning: EditorView's effect writes waveformView's `startSample`/`samplesPerPixel` from outside the owning module (found by H-68's recording harness) | S | done |
| H-84 | EQ graph remainder (T-409): live spectrum overlay, keyboard-operable nodes (SPEC-015 AC-19), expanded view | S | done |
| H-85 | Noise reduction UI remainder (T-504): profile graph, Clear Noise Print, Ctrl+Shift+P | S | done |
| H-86 | EQ mouse gestures vs SPEC-015 §2.6.4: double-click should reset a band (it toggles), and the wheel should step HP/LP slope (H-84's open question) | S | done |
| H-87 | Noise profile graph polish: hover readout, the no-output-device state, distinguishable legend swatches (H-85's deferrals) | S | done |
| H-88 | A shared test helper for pushing a live VXSA frame into a mounted component, so live-derived UI can be asserted rather than only its absence (H-87's open question) | S | done |
| H-89 | RELEASE BLOCKER, owner-reported: the AppImage bundles libpipewire without its spa plugins, so it fails on every non-Ubuntu distro (flood of pw.loop errors) — stop bundling it and guard against it | S | done |
| H-90 | RELEASE BLOCKER, owner-reported: the AppImage bundles libwayland-client, so WebKitWebProcess aborts and the window renders empty on a newer Wayland stack — denylisted (found via the upstream excludelist, which H-89's audit had not consulted) | S | done |
| H-96 | Owner-reported: the export UI hangs at "exporting" after the export has actually finished — `job_progress` is subscribed *after* the job starts, so a fast job's terminal event is lost (same pattern in normalize, LUFS normalize and bake) | S | done |
| H-98 | The unreproduced half of the export freeze: Cancel unresponsive, SIGTERM ignored, 72% CPU — needs a real-app repro harness and a profile at the moment of the freeze (H-96 fixed the lost event but could not reproduce the spin) | S | done |
| H-91 | Explain My Voice: frozen analysis snapshot, harmonic/peak reasoning, octave-error guard, findings model (engine only) | O | done |
| H-92 | Explain My Voice: the modal and the annotated log-frequency graph (raw + smoothed envelope, F0/harmonics, voice bands, hover, responsive) | S | done |
| H-93 | Explain My Voice: collision-aware annotation layout solver (pure geometry) | S | done |
| H-94 | Explain My Voice: finding prose, engineering summary, conservative EQ advice — measurement separated from interpretation | O | done |
| H-95 | Explain My Voice: verification against real voices (the owner's 8 cases + their own recording) | S | done |
| H-97 | Show pitch confidence in the diagnostics panel when an F0 estimate is shaky (H-91 follow-up) | H | done |
| H-99 | Align the diagnostics panel's hint wording with H-94's principles (no air boost to flatten a voice; "harsh" only on substantial evidence) | S | done |
| H-100 | A wall-clock timing assertion inside `just check` (VXTC frame budget) is flaky in debug builds under load — move it to the bench suite | H | done |
| H-103 | `rfd`'s gtk3 backend spawns a second, permanent GTK main loop the first time a native dialog opens — sharing one GMainContext with the window's own loop. Suspected in the owner's freeze (which followed a save dialog), not proven. Reproduce with the harness, then consider xdg-portal | S | todo |
| H-104 | The Explain modal's voice profile shows 2 of its 6 rows on a desktop-height dialog — fit it without scrolling, and decide whether phone-width charts should carry short cards | H | done |
| H-105 | Documentation refresh #2: everything since H-78 — Explain My Voice, octave-guarded pitch, the changed advice, loop/selection/import behaviour, export reliability, the AppImage bundling rules | S | done |
| H-106 | OWNER REQUEST: make the tours teach the app — noise and punch have one step each today, and a tour for Explain My Voice is missing | S | done |
| H-107 | OWNER REQUEST: a Help section inside the app (searchable, offline) covering how to use it and how to install third-party plugins — today Help has only About, Tours and Shortcuts | S | done |
| H-108 | OWNER-REPORTED, live repro on v0.4.0: Explain My Voice sits on "Analyzing…" forever on an unsaved recording while the WebView's JS thread spins at ~91 % — fix the cause, and show real progress with Cancel (owner request) | S | in progress |
| H-109 | OWNER-REPORTED: a selection drag released over another panel never ends and keeps following the mouse — the waveform only captures the pointer for marker drags | H | todo |
| H-110 | OWNER-REPORTED: dragging any control inside a rack slot (EQ nodes, sliders, transfer-graph handles) drags the whole slot instead — `draggable=true` is on the entire card, not the grip | S | in progress |
| H-111 | OWNER REQUEST: EQ graph usability — let the rack grow past its 480px/35% cap, make the expanded view discoverable, a cursor readout, node tooltips (freq/gain/Q), and right-click to add and delete bands (deferred since S3-07) | S | todo |
| H-112 | OWNER REQUEST: the input meter should match the vertical output meter, with a selectable floor of −60 / −80 / −120 dBFS (today a small horizontal bar fixed at −60) | S | todo |
| H-113 | OWNER-REPORTED: on the real WebGL2 renderer a selection is flat opaque gray, hiding the spectrogram entirely — and H-79's colours were only ever verified on Canvas2D, because the preview harness defaults to it while the app defaults to WebGL2 | S | in progress |
| H-114 | OWNER-REPORTED: while recording, the waveform lags ~10 s behind the voice, so it looks like nothing is recording — should grow live and smoothly like Audition/Audacity | S | in progress |
| H-115 | OWNER REQUEST: Explain My Voice much bigger (screen-relative, maximisable), an Annotations toggle, EQ Advice that says when there is nothing to show, and an export (image + report) to send to an engineer | S | todo |
| H-116 | CORRECTNESS: when a take's pitch range is too wide to resolve H2, the report calls the strongest peak "a resonance of the voice or of the room" — though it lines up with H2 inside the measured range. "Can't tell" must not become a false negative | S | in progress |
| H-117 | OWNER-REPORTED: the Spectrum Inspector has no legend (its dashed gray curve is the room tone, unlabelled), and Explain My Voice is not in it — add both, and open Explain instantly from the Inspector's existing Average | S | in progress |
| H-118 | `Gallery.test.ts` timed out at vitest's 5 s default under a load average near 40 — it renders the whole gallery in three themes, synchronously; given a 30 s ceiling after interleaved runs proved it load, not a regression | H | done |
| H-101 | The dashed EQ-suggestion overlay needs a way to evaluate a filter that isn't in the rack yet — backend preview, since SPEC-015 AC-17 forbids the UI evaluating filters (H-92 escalated rather than guessing) | S | done |
| H-102 | The Explain modal is correct but cramped: the graph is the smallest element, annotation text truncates mid-sentence, cards overrun the plot — layout pass against the owner's reference image | S | done |
| H-62 | Sandbox event-ring overflow is silent (H-55 open question): `HostEnd::push_event` / the proxy drop events when the ring is full (counted in `events_dropped`, never surfaced) — another timing-dependent divergence under heavy automation; surface it as a notice/slot status and test it | S | done |
| H-56 | Insert silence + cross-document clipboard (T-302 remainder, SPEC-008) | S | done |
| H-57 | Markers: kinds in the core model, drag, region→selection (T-303 remainder, SPEC-009) | S | done |
| H-58 | Dynamics part 2: activate Expander + AutoGate, real look-ahead latency (T-407 remainder, SPEC-016 part 2) | O | done |
| H-63 | `TransferCurve` module-api extension (H-58): the last blocker for SPEC-016 AC-17's transfer graph, SPEC-013's gate UI and T-410's custom Dynamics panel + VXMT telemetry | O | done |
| H-64 | Marker follow-ups (H-57): auto-scroll while dragging past the canvas edge (SPEC-009 §2.5 `drag_autoscroll_rate`), SPEC-009 §2.13 case 3 — a WAV cue set differing from the sidecar should win with a notice (credited to T-306 but unimplemented), and the deferred panel work (filter, sort, virtualization, Delete All/Filtered, rename shortcut) | S | done |
| H-65 | Close SPEC-016 AC-18 properly (H-58): a `#[doc(hidden)]`/cfg(test) state accessor on Dynamics (and NoiseGate) so the 120 s silence test can assert no subnormals in the module state, not just in the output | S | done |
| H-66 | Waveform right-click context menu for the edit ops (SPEC-008 §2.11 — never built by S2-01: Cut/Copy/Paste/Delete/Trim/Silence/Insert Silence), and gate `edit_copy` against a running normalize/paste job like its six siblings | S | done |
| H-59 | Engine/recording/IPC/UI hardening audit (T-105/T-106/T-108/T-109 remainder): matrix vs SPEC-001/002/003 + ADR-003, fill real gaps | O | done |
| H-67 | Notice actions (H-59, SPEC-002 AC-7): notices have no action affordance anywhere — add `Notice.action` + a button in Banner/Toast, first user being "Go to first" jumping to the first dropout marker | S | done |
| H-68 | UI responsiveness budget (H-59, SPEC-002 AC-8): no harness asserts "no frame over 100 ms" during a 12 s capture stall — build it on the T-704/H-43 frame-timing tooling and record the numbers | S | done |
| H-69 | Case-insensitive filesystem collisions break the Windows/macOS build (H-61): `menubar.svelte.ts` vs `MenuBar.svelte`, `shortcutsDialog.svelte.ts` vs `ShortcutsDialog.svelte` — rename, fix the import sites, and add a `just check` guard that fails on any path colliding case-insensitively | S | done |
| H-60 | Save pipeline / peaks / waveform renderer audit (T-201/T-203/T-205 remainder): matrix vs SPEC-005/006/009/018, fill real gaps | S | done |
| H-61 | Windows + macOS installers from CI (T-705 remainder), labelled unverified | S | done |
| H-50 | Test robustness: de-flake `src-tauri normalize::tests::start_peak_job_runs_end_to_end_and_reports_done_with_a_result` (waits for Done then asserts the Result event emitted after it — poll instead), re-check the output-callback worst case on an idle machine (T-704/H-43 open question) | S | done |
| H-51 | Doc/code truth gaps found by T-706: refresh ADR-001 §2/§4, ADR-002 §1 (no rayon pool, 60 Hz telemetry, 65536 monitor ring), ADR-004 §1/ADR-001 §6 (sessions use ProjectDirs data_dir = Windows roaming, modules use app_local_data_dir — decide and align), ADR-008 §5/§8 superseded lines; document or fix hidden SPEC-016/SPEC-013 params (AutoGate/Expander/look-ahead inert), the output-meter vs analyzer-tap dry-monitor difference, stale src-tauri comments | S | done |
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
| T-105 | Engine core: control thread, RT callbacks, transport (Stop→play start), reader/prefetch, resampling, heard clock, 60 Hz telemetry | W2 | O | T-101, T-102, T-103 | done (hardening remainder: H-59) |
| T-106 | Recording: crash-safe WAV, peaks stream, take → undoable edit | W3 | O | T-105 | done (hardening remainder: H-59) |
| T-107 | Monitoring off/dry/through-rack, drift-corrected in→out ring | W3 | O | T-105 | done |
| T-108 | IPC layer: M1 commands/events, VXTM/VXRP channels, binary decoders + golden fixtures, clock sync | W3 | S | T-104, T-105 | done (hardening remainder: H-59) |
| T-110 | Bench harness (divan, callback-time histogram) | W3 | S | T-105 | done |
| T-109 | UI: device settings, transport bar, meter bridge, live recording waveform | W4 | S | T-106, T-107, T-108 | done (hardening remainder: H-59) |

## M2 — Editor view
| ID | Title | Wave | Tier | Deps | Status |
|---|---|---|---|---|---|
| T-200 | M2 spec wave: SPEC-005 formats, SPEC-006 waveform view, SPEC-007 spectral & analyzer | W0 | O/S | M0 | done |
| T-201 | Save pipeline: WAV writer 16/24/32f, TPDF dither, clip policy, cue/adtl markers, atomic save job, FLAC encode | W1 | S+OR | M1 | done (hardening remainder: H-60) |
| T-202 | Import pipeline: symphonia decode (WAV variants, FLAC, MP3, M4A, Ogg), downmix, session import job, CLI convert/markers | W1 | S+OR | M1 | done (import job UI + channel-choice dialog → T-209) |
| T-203 | Per-chunk peak pyramid + binary IPC | W1 | S+OR | M1 | done (hardening remainder: H-60) |
| T-204 | STFT spectrogram tile service | W1 | O | M1 | done |
| T-205 | Waveform renderer: zoom/scroll, amplitude zoom, rulers, extrapolated playhead | W2 | S | T-203 | done (hardening remainder: H-60) |
| T-206 | Selection model, time formats, zero-crossing snap | W3 | S | T-205 | done |
| T-207 | Spectrogram renderer, log/linear, split view | W3 | S | T-204, T-205 | done |
| T-208 | Live output analyzer: post-rack tap, FFT worker, 1/24-oct bands, VXSA, bottom-dock UI | W3 | S+OR | T-205 | done (follow-ups → H-16) |
| T-209 | File menu & dialogs: Open, Save, Save As, downmix dialog, save format, clip prompt, unsaved-changes prompt, import progress | W3 | S | T-201, T-202, T-205 | done (gaps → H-20) |

## M3 — Editing & history
| ID | Title | Wave | Tier | Deps | Status |
|---|---|---|---|---|---|
| T-300 | M3 spec wave: SPEC-008, SPEC-009, SPEC-010, SPEC-018, SPEC-022 | W0 | O | M0, SPEC-005 | done |
| T-301 | Undo/redo, resident memory budget, edit journal, crash recovery | W1 | O | M2 | done (follow-ups → H-17) |
| T-302 | Cut/copy/paste/delete, trim, silence, insert silence, clipboard | W2 | S+OR | T-301 | done (remainder: H-56) |
| T-303 | Markers: kinds, add/region, rename, drag, delete, navigation, Markers panel | W2 | S | T-301 | done (remainder: H-57, H-64) |
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
| T-407 | Dynamics B: expander + AutoGate | W2 | O | T-403 | done (remainder: H-58, H-65) |
| T-408 | Noise gate (shared envelope/hysteresis) | W3 | O | T-407 | done (via S3-02) |
| T-409 | EQ graph UI (analyzer + ResponseCurve) | W3 | S | T-402, T-405, T-208 | done (remainder: H-84) |
| T-410 | Dynamics UI + gain-reduction meter (Telemetry) | W3 | S | T-405, T-407 | done (remainder: H-63, H-77) |

## M5 — Noise reduction
| ID | Title | Wave | Tier | Deps | Status |
|---|---|---|---|---|---|
| T-500 | M5 spec wave: SPEC-014 noise reduction (module, noise print capture, UI) | W0 | O | M0 | done |
| T-501 | Offline NR algorithm (decision-directed Wiener + smoothing) + goldens | W1 | O | M4 | done (via S3-04) |
| T-502 | Noise print capture + profile storage (state blob) | W1 | S | M4 | done (via S3-04 + S3-06) |
| T-503 | Streaming NR module, latency_changed on FFT size | W2 | O | T-501, T-502 | done (via S3-04) |
| T-504 | NR UI: capture, profile graph, output-noise-only | W3 | S | T-503 | done (remainder: H-85) |

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
| T-704 | Performance tuning vs targets | W1 | O | M6 | done |
| T-705 | AppImage/deb, Win/mac build docs, README/user guide | W2 | S | T-701..T-704 | done (Win/mac installers documented, not built) |
| T-706 | Complete architecture & developer docs (owner req.): docs hub, C4/crate/threading/sequence Mermaid diagrams, data/IPC/DSP/plugins/UI docs, contributing guide, ADR index, docs check script | W3 | O | T-809 | done |
| T-707 | Docs for everyone (owner req.): what-is / how-it-works with Mermaid, complete task-based user guide, FAQ, glossary with plain-language analogies, friendly README | W4 | S | T-706, T-708, T-709 | done |
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
| T-811 | VST2 adapter — **dropped** 2026-09-17 (owner): Steinberg no longer licenses the VST2 SDK (ADR-007 §7) | W5 | O | — | dropped |

## M9 — Native plugin editors
| ID | Title | Wave | Tier | Deps | Status |
|---|---|---|---|---|---|
| T-901 | Plugin GUI as floating window owned by sandbox (HWND/NSView/X11-XWayland) | W1 | O | M8 | done |

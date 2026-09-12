# PROMPT.md — Master Brief (v1)

> This is the founding prompt for every AI agent working on this project.
> The orchestrator reads it in full; subagents read the sections referenced by their ticket.
> Decisions marked **LOCKED** were made by the product owner and must not be changed without asking.
> Product name: **PowerVoice** (chosen by the owner at the M0 checkpoint; formerly the working name "VoxEdit").

---

## 1. Mission

Build a cross-platform (Windows, macOS, Linux) desktop audio editor that is a focused alternative
to Adobe Audition **for simple voice-over editing**: record mono voice, clean it, shape it, hit
loudness targets, export. Depth over breadth — every feature that exists must be correct, fast,
and measurably verified.

## 2. Locked decisions

| Area | Decision |
|---|---|
| Stack | **Tauri 2** desktop app. **Rust** for audio I/O, DSP, file I/O, document model. **Svelte 5 + TypeScript** UI. |
| Editing model | **Single-file waveform editor** (one mono file open at a time, like Audition's Waveform view). No multitrack in v1. |
| Effects workflow | **Non-destructive effects rack** processed in real time on playback, monitoring and export. |
| Normalize favorites | **One-click destructive** actions on selection (or whole file if no selection), fully undoable. |
| Formats | Open/save **WAV 16/24/32f**; **FLAC** import/export; **MP3 export**; **import MP3/M4A(AAC)/OGG(Vorbis)**. |
| Recording | Mono. Default **48 kHz / 24-bit**, user-selectable per new file. Internal processing **32-bit float**. |
| Monitoring | Selectable **off / dry / through rack**; default off. |
| Persistence | Plain audio file + **sidecar** `name.vo.json` (rack, markers, noise profile, view state) + autosave/crash recovery. |
| Noise reduction | **Spectral noise-print** (Audition-style capture profile → spectral subtraction/Wiener gain). No ML in v1. |
| EQ | **Audition-style parametric**: HPF + LPF (variable slope), low shelf, high shelf, 5 peaking bands, draggable graph with live spectrum. |
| Loudness | Peak normalize favorites **+ LUFS normalize + ACX check + true-peak limiter**. |
| UI | **Audition-like dark layout**. English strings, **i18n-ready** (all strings in a locale file). |
| Shortcuts | **Audition-compatible defaults** (not remappable in v1). |
| Repo | **Local git**, one reviewed commit per ticket. No remote, no CI service in v1 (local check scripts instead). |
| Test platform | Product owner tests on **Linux (Arch, PipeWire)** only. Windows/macOS must compile and follow platform-neutral code paths, but are unverified until someone tests them — track this as a known risk. |
| Orchestration | Main Claude session = **orchestrator** using the Agent tool; subagents per ticket. ~~Pause for owner review after each milestone~~ — **after the M0 checkpoint the owner authorized autonomous execution to the end of the tickets, without checkpoints or questions** (2026-09-12, MEMORY D-021). |
| Module system | Every rack effect implements one **Module API** (processor + parameter schema + presets + latency report). Built-in modules are compiled in for v1; the same API later loads **separately installed module packages**. |
| External plugins | Host **VST3, CLAP, LV2, VST2 (legacy)** and **REAPER JSFX** (via `ysfx`). Delivered as **M8**, after the core editor, but the rack is plugin-shaped from day one. |
| Plugin isolation | External plugins run **out-of-process** in a sandbox host with shared-memory audio; a crashing plugin never takes down the editor or a recording. |
| Plugin UI | **Generic auto-generated parameter UI first**; native plugin editor windows follow in **M9** (X11/XWayland on Linux). |
| Plugin install | **Scan standard OS plugin paths + custom folders**, "Install module…" from file, plugin manager (enable/disable, rescan, blocklist for plugins that fail scanning). |

## 3. v1 feature scope

### 3.1 Audio I/O & recording
- Enumerate audio hosts/devices (cpal); choose **input device + input channel** (e.g. channel 1 or 2 of a 2-in interface) and **output device**. Refresh on hot-plug.
- Pre-record input level meter with clip indicator; record into new file or at cursor (insert/overwrite).
- **Punch-in**: re-record over a selected region with configurable pre-roll/post-roll.
- Transport: play/stop/pause, record, loop selection, return-to-start, playhead follow.

### 3.2 Editor view
- **Waveform** view (min/max peak pyramid, smooth zoom 1 sample → whole file, vertical amplitude zoom, dB/percentage ruler).
- **Spectral display** (STFT spectrogram, log/linear frequency toggle, configurable FFT size, colormap), toggleable/split with waveform like Audition.
- Time selection, zoom/scroll, time ruler (timecode / samples / seconds), optional snap to zero-crossing.
- Level meters (peak + RMS, post-rack), live output spectrum analyzer.

### 3.3 Editing (destructive, undoable)
- Cut / copy / paste / delete; trim to selection; silence; insert silence.
- **Normalize favorites**: −1 dB, −0.1 dB, −3 dB (peak); plus custom value.
- **LUFS normalize** favorites: −16, −19, −23 LUFS (+ custom). Mono measured per ITU-R BS.1770 (no dual-mono +3 dB compensation by default; option in settings).
- **Bake rack**: render the current rack into the selection/file (undoable), then rack resets.
- Unlimited undo/redo within a memory budget (edit list over immutable chunked buffers — engine designs this).
- **Markers**: add, rename, navigate, markers list panel; stored in sidecar (and WAV `cue` chunk on export).

### 3.4 Effects rack (non-destructive)
Every rack slot holds a **processor** through the Module API (§3.7): a built-in module now, an external plugin from M8.
Built-in modules (reorderable, per-module bypass, whole-rack bypass for A/B, per-module presets, full-rack presets):
1. **Gain**
2. **Noise Gate** — threshold, attack, hold, release, range (attenuation dB), hysteresis, optional sidechain HPF.
3. **Noise Reduction** — capture noise print from selection; reduction (dB), reduce-by %, spectral smoothing, attack/release (spectral decay), FFT size, "output noise only" audition toggle. Reports its latency; engine compensates.
4. **Parametric EQ** — see §2. RBJ-cookbook biquads; HPF/LPF 6–48 dB/oct (cascaded Butterworth); graph shows exact computed response.
5. **Dynamics** (Audition "Dynamics" style, 4 toggleable sections): **AutoGate** (threshold, attack, release, hold), **Compressor** (threshold, ratio, attack, release, makeup), **Expander** (threshold, ratio), **Limiter** (threshold, attack, release); peak/RMS detection, optional look-ahead, gain-reduction meter.
6. **True-Peak Limiter** — ceiling in dBTP (default −1.0), look-ahead, release; 4× oversampled detection.

Rack applies to playback, monitoring ("through rack" mode) and export. Parameter changes are smoothed (no zipper noise).

### 3.5 Loudness & delivery
- **Loudness meter/analysis**: integrated LUFS, short-term, momentary, LRA, sample peak, true peak.
- **ACX check** (analyzes processed output by default): RMS between −23 and −18 dB, peak ≤ −3 dBFS, noise floor ≤ −60 dB (quietest 500 ms window), sample rate/format hints; pass/fail report.
- **Export**: WAV (16/24/32f), FLAC, MP3 (CBR/VBR; ACX preset = 44.1 kHz 192 kbps CBR) with high-quality resampling and TPDF dither when reducing bit depth. Export renders the rack.

### 3.6 App shell
- Audition-like dark layout: toolbar + transport (top), editor (center, waveform/spectral), effects rack (right), markers + properties (left/bottom), meters (bottom).
- Settings: devices, buffer size, default format, monitoring mode, loudness mono compensation, theme accents.
- Audition-compatible shortcuts (Space play/pause, Shift+Space play from start, Shift+R record (provisional), Ctrl+Z / Ctrl+Shift+Z, M add marker, etc. — SPEC-019 lists the full map; owner decision D-014).
- Autosave + crash recovery; recent files.

### 3.7 Module & plugin system
- **Module API** (crate `module-api`): the single contract for anything in the rack — `prepare(sample_rate, max_block)`, real-time-safe `process(block)`, `reset()`, `latency_samples()`, `tail_samples()`, a declarative **parameter schema** (id, name, unit, range, default, taper, automation-smoothing), state save/load, presets. Built-in modules and external-plugin adapters both implement it; the rack, latency compensation, presets and the **generic parameter UI** only ever see this API.
- **Installable module packages** (later): the ADR decides the binary format — preferred direction is to reuse **CLAP as the packaging ABI** rather than invent a new one, so a separately installed module is a CLAP bundle that also exposes our parameter/preset metadata.
- **External plugin hosting (M8)**: format adapters for **CLAP, VST3, LV2, VST2, JSFX** behind the Module API. Mono in/out; for stereo-only plugins, feed dual-mono and take the left channel back (documented per adapter).
- **Sandbox (M8)**: a separate `plugin-sandbox` process per plugin (or per rack chain — the ADR decides) talking to the engine over shared-memory ring buffers + a control channel; watchdog detects hangs/crashes, the engine substitutes silence or bypass, the user sees a notice, the plugin is flagged. Added latency is reported through `latency_samples()`.
- **Discovery (M8)**: scan standard per-OS paths + user folders in the sandbox process (a crashing plugin can't kill the scan), cache results, blocklist plugins that fail.
- **Plugin manager UI (M8)**: list/enable/disable, rescan, custom folders, "Install module…" from file.
- **Native plugin editors (M9)**: open the plugin's own GUI in a separate native window (Windows HWND, macOS NSView, Linux X11 via XWayland).
- **Legal/licensing checkpoint**: VST2 hosting needs reverse-engineered headers because Steinberg no longer licenses the VST2 SDK. **Stop and get owner confirmation before implementing the VST2 adapter.** Record the licenses of the VST3 SDK, LV2, CLAP, `ysfx`/WDL and LAME in an ADR.

### 3.8 Out of scope for v1
Multitrack, fades, AU plugins, online module catalog, ML denoise, spectral *editing* (display only), stereo editing, batch processing, remappable shortcuts, cloud/sync, video.

## 4. Architecture constraints

```
repo/
├── PROMPT.md  MEMORY.md  CLAUDE.md
├── specs/            SPEC-NNN-*.md (source of truth for behavior)
├── tickets/          T-NNN-*.md (units of work) + BOARD.md
├── crates/
│   ├── module-api/   the Module API: processor trait, parameter schema, state/presets (no deps on engine)
│   ├── dsp/          pure DSP: no I/O, no threads, no allocation inside process(); unit + golden tests
│   ├── modules/      built-in rack modules (gain, gate, NR, EQ, dynamics, TP limiter) on module-api + dsp
│   ├── rack/         pure chain host (no cpal): slots, host bypass, param routing, latency compensation, offline render — shared by engine, export, bake, CLI
│   ├── engine/       audio backends (cpal + fake), audio threads, transport, recording, monitoring, reader/prefetch
│   ├── plugin-host/  (M8) format adapters CLAP/VST3/LV2/VST2/JSFX, scanner, sandbox IPC client
│   ├── plugin-sandbox/ (M8) out-of-process plugin host binary
│   ├── io/           decode/encode (WAV, FLAC, MP3, AAC, Vorbis), resampling, dither
│   ├── project/      document model (disk-backed chunk store, Arc snapshots, piece table, edit journal), edit ops, undo, markers, sidecar, autosave/recovery
│   ├── testkit/      signal generators, measurements (peak/RMS/LUFS/TP), golden-file helpers (dev-dependency)
│   └── cli/          `powervoice-cli`: gen / render --rack / analyze / bench — DSP acceptance tool
├── src-tauri/        thin command/event layer only — no business logic
└── ui/               Svelte 5 + TS; Canvas/WebGL renderers for waveform & spectrogram
```

- **Real-time safety**: the audio callback never allocates, locks, logs, or blocks. Use lock-free SPSC ring buffers (e.g. `rtrb`) and atomics; UI→audio parameter changes go through a lock-free queue with smoothing.
- **IPC**: bulk data (peaks, spectrogram tiles) as raw binary (Tauri raw responses / channels), never JSON arrays of floats. Meters/playhead as events at ~30–60 Hz.
- **Rendering**: waveform from precomputed peak pyramid; spectrogram computed in Rust as tiles (u8 magnitude), colored and drawn on the GPU (WebGL) in the UI.
- Candidate crates (the architect confirms each choice in an ADR): `cpal`, `rtrb`, `hound`, `symphonia`, `flacenc` or equivalent, `mp3lame-encoder` (LGPL — note the licensing), `rubato`, `realfft`/`rustfft`, `ebur128` (as a reference for tests or for production use).
- **Performance targets**: open a 60-min 48 kHz mono WAV in < 3 s with the waveform shown progressively; 60 fps scroll/zoom; playback start < 50 ms; full rack at 48 kHz < 20% of one core on a mid-range laptop; noise-reduction latency ≤ 50 ms.

## 5. Spec-driven development

1. **Specs first, just-in-time.** Each milestone opens with a spec wave (W0); the owner approves those specs at the previous milestone's checkpoint. Every feature has `specs/SPEC-NNN-name.md` containing: purpose, UX description, parameters (name, unit, range, default), algorithm notes, **acceptance criteria as Given/When/Then with numeric tolerances**, and a test plan.
2. **Tickets reference specs.** `tickets/T-NNN-name.md` contains: spec refs, scope, files/crates touched, dependencies, **model tier**, Definition of Done, and the acceptance tests to write.
3. **Tests encode the spec.** DSP acceptance = golden/numeric tests against synthetic signals. Examples of the precision expected:
   - Peak normalize −1 dB → resulting sample peak −1.00 dBFS ± 0.01 dB.
   - LUFS: 1 kHz sine at −20 dBFS (mono, 48 kHz) and EBU Tech 3341/3342 cases → within ± 0.1 LU.
   - True-peak limiter at −1 dBTP → no 4×-oversampled peak > −0.9 dBTP on stress signals.
   - EQ: magnitude response matches analytic biquad response within ± 0.1 dB at 1/12-octave points.
   - Compressor: static curve for steady sine within ± 0.5 dB of threshold/ratio math.
   - Noise reduction: tone bursts + white noise at −50 dBFS, 12 dB reduction → noise floor drops ≥ 10 dB while tone level changes < 0.5 dB.
4. **Definition of Done** (every ticket): `cargo fmt --check`, `cargo clippy -D warnings`, `cargo test` for the workspace; `svelte-check` + `vitest` for the UI; the ticket's acceptance tests pass; DSP/engine tickets pass an Opus review; one commit `T-NNN: <summary>`; MEMORY.md updated by the orchestrator.

## 6. Orchestration protocol

- **Orchestrator** (main session, Opus 5): owns BOARD.md and MEMORY.md, dispatches tickets in dependency-ordered **waves**, runs independent tickets in parallel in **git worktrees**, reviews every result, merges, commits, and **pauses after each milestone** for owner review with a demo build + summary.
- **Model tiering** (never above Opus 5):
  | Tier | Model | Use for |
  |---|---|---|
  | Simple | **Haiku 4.5** | scaffolding, config, boilerplate, docs, i18n string extraction, simple glue |
  | Standard | **Sonnet 5** | UI components, IPC wiring, file I/O, editing ops, test writing, sidecar/autosave |
  | Complex | **Opus 5** | architecture/ADRs, real-time engine & threading, DSP algorithms (NR, dynamics, EQ, LUFS, true-peak, resampling), undo model, Module API design, plugin format adapters, sandbox IPC, all DSP/engine/plugin code reviews |
- **Subagent contract**: read PROMPT.md §-refs + the ticket + referenced specs + MEMORY.md; stay inside the ticket's scope; don't edit MEMORY.md or BOARD.md (report notes back instead); return a summary of changes, test results and open questions.
- **MEMORY.md** holds: current milestone & status, decisions log (ADR-lite), conventions, gotchas/learnings, known risks. The orchestrator curates it after every ticket.
- **Escalation**: anything that would change a LOCKED decision, add scope, or add a dependency with licensing implications → stop and ask the owner.

## 7. Milestones (authoritative ticket list: `tickets/BOARD.md`)

- **M0 Foundations** — install toolchain (rustup/cargo, just, Tauri CLI; Node 26 + WebKitGTK/ALSA/PipeWire already present), git, Cargo workspace, Tauri 2 + Svelte 5 scaffold, check scripts (`just check`), CLAUDE.md, MEMORY.md, spec/ticket templates, ADRs, **Module API v0**, test harness + `powervoice-cli`, platform spike (WebKitGTK rendering, Wayland input), M1 specs.
- **M1 Core engine, recording & playback** — document model (chunk store/snapshots), rack core in the signal path (Gain module, offline render), app infrastructure (logging, errors, settings, keymap registry), device/channel selection, meters, mono recording, playback, transport, monitoring off/dry/through-rack.
- **M2 Editor view** — WAV open/save + cue markers, importers, per-chunk waveform pyramid, zoom/scroll/selection, spectral display, analyzer.
- **M3 Editing & history** — clipboard ops, trim/silence/insert silence, undo/redo, markers, punch-in, normalize favorites, sidecar + autosave/recovery.
- **M4 Effects modules & rack UI** — latency compensation + latency changes, generic parameter UI, gain, noise gate, parametric EQ + graph, dynamics, true-peak limiter, presets, monitoring through rack.
- **M5 Noise reduction** — noise print capture + spectral NR module.
- **M6 Loudness & export** — loudness analysis, LUFS normalize, ACX check, bake rack, WAV/FLAC/MP3 export with resampling/dither.
- **M7 Polish & packaging** — full shortcut map, i18n extraction pass, settings, Linux packages (AppImage/deb), documented Windows/macOS build steps.
- **M8 External plugins** — sandbox process + shared-memory IPC, scanner + blocklist, CLAP → VST3 → LV2 → JSFX adapters (VST2 after the owner's legal sign-off), plugin manager UI, "Install module…", plugin state saved in sidecar/presets.
- **M9 Native plugin editors** — the plugin's own GUI in native windows (Win/macOS/X11-XWayland).

## 8. Planning-phase deliverables

1. Architecture overview + ADRs for key choices (crates, undo model, IPC data paths, threading, **Module API + installable-module ABI**, **plugin sandbox IPC**, **plugin-format licensing**).
2. Complete `specs/` set covering §3, with acceptance criteria per §5.
3. `tickets/` set with dependency graph, model tier and estimated size per ticket; `BOARD.md`.
4. Initial `MEMORY.md` and `CLAUDE.md`.
5. Milestone plan with the owner-review checkpoints.

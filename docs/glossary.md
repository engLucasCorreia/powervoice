# Glossary

The words PowerVoice's documentation, code and UI use, in alphabetical order. Each entry starts
with a **plain-language line** anyone can follow, then gives the precise meaning and points to
where it lives. The non-technical guide (T-707) extends this page; keep one entry per term.

Jump to: [A](#a) · [B](#b) · [C](#c) · [D](#d) · [E](#e) · [F](#f) · [G](#g) · [H](#h) ·
[I](#i) · [J](#j) · [L](#l) · [M](#m) · [N](#n) · [O](#o) · [P](#p) · [R](#r) · [S](#s) ·
[T](#t) · [U](#u) · [V](#v) · [W](#w) · [Z](#z)

## A

**A/B (whole-rack A/B)**
- *Plain:* a switch that lets you hear your voice with and without all the effects, to compare.
- Listening-only: it swaps the rack output for the latency-matched dry signal with a crossfade
  (`vox_rack::LiveRack`, `RackCommand::SetAb`). Exports always render the rack (owner decision
  D-019). See [dsp.md](architecture/dsp.md#the-live-signal-chain).

**ACX check**
- *Plain:* a pass/fail test against the audiobook platform ACX/Audible's technical rules.
- RMS between −23 and −18 dB, sample peak ≤ −3 dBFS, noise floor (quietest 500 ms window)
  ≤ −60 dB, all inclusive; `vox_dsp::acx::evaluate`. Runs on the rack-processed audio by default.
  See [dsp.md](architecture/dsp.md#loudness-acx-and-normalize).

**Adapter**
- *Plain:* a translator that makes a third-party plugin look like a built-in effect.
- A `ModuleFactory`/`Module` implementation that wraps an external format. In PowerVoice every
  adapter is out-of-process: `vox_plugin_host::SandboxFactory` → `ProxyModule` → a
  `powervoice-sandbox` process running a format backend (CLAP, VST3, LV2, JSFX). See
  [plugins.md](architecture/plugins.md).

**ADR (Architecture Decision Record)**
- *Plain:* a short document that records a design decision and why it was made.
- `docs/adr/ADR-NNN-*.md`, accepted at the M0 checkpoint and amended as features landed. Index:
  [adr/README.md](adr/README.md).

**Audio callback / audio thread**
- *Plain:* the tiny piece of code the operating system calls hundreds of times a second to fetch
  the next sound to play or deliver the sound just recorded.
- The cpal output and input callbacks (`crates/engine/src/output.rs`, `input.rs`). They must
  never allocate, lock, log, do I/O or block. See
  [runtime.md](architecture/runtime.md#real-time-rules).

## B

**Bake (Bake Rack)**
- *Plain:* permanently apply the current effects to the audio, as one undoable step.
- Renders the rack offline into the selection (or the whole file) through
  `vox_engine::bake::render_document_range`, commits one `history.bake` edit carrying the
  pre-bake rack, then resets the rack. See
  [runtime.md](architecture/runtime.md#apply-the-rack-bake).

**Blocklist**
- *Plain:* a list of plugin files PowerVoice won't load because they crashed or hung while being
  checked (or because you blocked them).
- `plugin-blocklist.json` in the cache folder (`vox_plugin_host::blocklist`): path + size +
  mtime + CRC-32, reason `Crashed` / `TimedOut` / `Manual`; an entry clears itself when the file
  changes. See [plugins.md](architecture/plugins.md#scanner-catalog-blocklist-and-health).

**Block (buffer)**
- *Plain:* a small batch of audio samples processed in one go.
- The frames handed to `process()` in one call. The rack splits callbacks into sub-blocks of at
  most `MAX_BLOCK` = 1024 frames; offline renders use 4096-frame blocks.

**Bypass**
- *Plain:* turn one effect off without removing it.
- Host-owned per-slot bypass with a 15 ms crossfade to the latency-matched dry path
  (`vox_rack::Slot::set_bypass`).

## C

**Canvas2D / WebGL2**
- *Plain:* two ways the app can draw the waveform and spectrogram on screen; WebGL2 uses the
  graphics card, Canvas2D is the simpler fallback.
- `ui/src/lib/render/` chooses per canvas (`rendererMode.ts`) and latches to Canvas2D on
  `webglcontextlost` (`glContext.ts`). See [ui.md](architecture/ui.md#renderers).

**Chunk / chunk store**
- *Plain:* the audio is kept on disk in fixed-size pieces that never change, so undo is cheap.
- 65 536-sample immutable f32 chunks in preallocated 64 MiB segment files, memory-mapped, with a
  CRC-32 and a peak pyramid per chunk (`vox_project::store::ChunkStore`). See
  [data.md](architecture/data.md#the-chunk-store).

**CLAP**
- *Plain:* a modern, open plugin format for audio effects.
- CLAP 1.2, hosted in the sandbox through hand-written bindings (`crates/clap-abi`); also the
  packaging ABI for installable PowerVoice modules (ADR-006). See
  [plugins.md](architecture/plugins.md).

**Composition root**
- *Plain:* the place where the app's parts are plugged together at start-up.
- `src-tauri/src/lib.rs::run` (the app) and `crates/cli` (the CLI): they build the module
  registry and the engine. See [overview.md](architecture/overview.md#composition-roots).

**Control thread**
- *Plain:* the engine's "manager" thread that talks to the UI and the audio thread.
- `vox-control` (`crates/engine/src/control.rs`), 60 Hz tick; owns the rack host, devices and
  transport. See [runtime.md](architecture/runtime.md#threads).

**Crash recovery**
- *Plain:* if the app or computer crashes, PowerVoice brings back your unsaved work next time.
- Journal replay of the session directory (`vox_project::recovery`, `Session::recover`). See
  [data.md](architecture/data.md#recovery) and
  [runtime.md](architecture/runtime.md#crash-recovery).

**Crossfade**
- *Plain:* a very short blend from one sound to another so there's no click.
- Rack edits use 15 ms linear crossfades (`XFADE_MS`); punch-ins use 10 ms equal-power fades.

## D

**dBFS / dBTP / LUFS / LU**
- *Plain:* units for how loud audio is. 0 dBFS is the loudest a digital file can hold; LUFS
  measures loudness the way people hear it.
- dBFS = decibels relative to full scale (sample values); dBTP = true peak, measured between
  samples with oversampling; LUFS = loudness units relative to full scale per ITU-R BS.1770;
  LU = a difference in LUFS (used for loudness range, LRA).

**Denormals (FTZ/DAZ)**
- *Plain:* extremely tiny numbers that make processors slow; the audio code turns them into
  zero.
- `vox_dsp::fp::DenormalGuard` sets flush-to-zero / denormals-are-zero on the audio callbacks,
  spectrogram workers and offline renders.

**Dither (TPDF)**
- *Plain:* a whisper of noise added when saving at a lower bit depth so quiet sounds stay smooth.
- Triangular-PDF dither with a seeded PCG32 generator, skipped for 4096-sample blocks that are
  already exact (`vox_dsp::dither`). 32-bit float is never dithered; MP3 export isn't dithered.

**Document / session**
- *Plain:* the recording you have open, plus PowerVoice's private working copy of it.
- The one open mono document is backed by a session directory (chunk store + journal + takes).
  The original file is never modified until you save. See [data.md](architecture/data.md).

**Drift servo**
- *Plain:* keeps the microphone and speaker clocks in step while you listen to yourself.
- A PI controller on the monitor ring's fill level (`vox_engine::drift::DriftServo`) that steers
  `MonitorResampler` by up to ±1000 ppm.

**Dropout**
- *Plain:* a gap where the microphone stopped delivering sound.
- Detected on the input callback's timestamps; short gaps are filled with silence and marked with
  a "Dropout N ms" marker; gaps over ~2 s stop the take (A-011).

**DSP**
- *Plain:* the maths that changes sound (filters, compressors, noise reduction...).
- Digital signal processing; lives in `crates/dsp` (pure, no I/O) and `crates/modules`.

**DTO**
- *Plain:* a plain data shape used to send information between the Rust core and the UI.
- Data-transfer object: Rust structs in `src-tauri/src/ipc/*_dto.rs` deriving `ts_rs::TS`, from
  which `ui/src/lib/ipc/bindings.ts` is generated. See [ipc.md](architecture/ipc.md#shared-types-ts-rs).

## E

**Epoch**
- *Plain:* a counter that tells the audio thread "the old audio in the queue is stale now".
- Transport starts and seeks bump an epoch; the output callback drops reader packets from older
  epochs (`crates/engine/src/output.rs`).

**Export**
- *Plain:* write a finished file (WAV, FLAC or MP3) with the effects applied.
- A job: offline rack render → optional f64 resampling → TPDF dither → encoder
  (`src-tauri/src/export.rs`). See [runtime.md](architecture/runtime.md#export).

**Extension (module extension)**
- *Plain:* an optional extra ability an effect can offer, such as drawing its EQ curve.
- Typed interfaces behind `Module::extension(ExtensionId)`: `Telemetry`, `ResponseCurve`,
  `NoiseProfile` (public) and `AdapterHealth`, `ParamText`, `PluginEditor` (host-internal). See
  [plugins.md](architecture/plugins.md#extensions).

## F

**FFT**
- *Plain:* a way of splitting sound into its frequencies.
- Fast Fourier transform (`realfft`/`rustfft` via `vox-dsp`), used by noise reduction, the
  spectrogram, the analyzer and diagnostics.

## G

**Generated section**
- *Plain:* a part of a document written by a script, so it can't go out of date.
- Text between `<!-- BEGIN GENERATED: name -->` markers, rewritten by `just docs`
  (`scripts/docs/check.py --write`) and verified by `just check`.

**Golden test**
- *Plain:* a test that compares output with a saved, known-good result.
- E.g. `crates/io/tests/golden_save.rs` (file hashes) and `crates/engine/tests/spectro.rs`
  (tile in `tests/data/`, rewritten with `POWERVOICE_BLESS=1`).

## H

**Hot-plug**
- *Plain:* plugging or unplugging a microphone or headphones while the app runs.
- Detected by polling every second on `vox-device-poll` (cpal has no device events).

## I

**i18n**
- *Plain:* preparing the app so its text can be translated.
- Every user-facing string is a key in `ui/src/lib/i18n/en.json`, rendered with `t()` / `tDynamic()`;
  the backend sends keys, never text. See [ui.md](architecture/ui.md#i18n).

**IPC**
- *Plain:* how the window (UI) and the audio core talk to each other.
- Tauri commands (request/reply), events (push) and channels (binary streams). See
  [ipc.md](architecture/ipc.md).

## J

**Job**
- *Plain:* a long task (export, analysis...) that runs in the background with a progress bar.
- A named thread per job in `src-tauri` reporting through the `job_progress` event with a
  `JobKind`. See [runtime.md](architecture/runtime.md#jobs).

**Journal**
- *Plain:* a diary where every edit is written down immediately, so work survives a crash.
- `journal.<gen>.jsonl`: one `<crc32>\t<json>` line per record, `fdatasync`ed before a command
  returns (`vox_project::journal`). See [data.md](architecture/data.md#the-journal).

**JSFX**
- *Plain:* REAPER's simple text-based effect scripts.
- Hosted through the vendored ysfx library (`crates/ysfx-sys`), in the sandbox only, unix only.

## L

**LAME**
- *Plain:* the MP3 encoder library PowerVoice uses if it's installed on your system.
- `libmp3lame`, loaded at run time with `libloading` (`crates/io/src/mp3.rs`, ADR-007).

**Latency / latency compensation**
- *Plain:* the small delay an effect adds; PowerVoice shifts everything so what you see and hear
  still lines up.
- Each module reports `latency_samples()`; the chain sums them; the playhead, pre-roll and
  offline renders compensate. See [dsp.md](architecture/dsp.md#latency).

**LTAS**
- *Plain:* the average tone colour of a whole recording.
- Long-term average spectrum (`vox_dsp::diagnostics::spectrum::Ltas`), computed by the Average
  mode of the analyzer (`src-tauri/src/spectrum.rs`).

**LV2**
- *Plain:* an open plugin format common on Linux.
- Hosted in the sandbox through `lilv`, loaded at run time; unix only.

## M

**Marker**
- *Plain:* a named bookmark (or range) in the recording.
- `{id, pos, len, name, kind}` in the snapshot; saved in the sidecar and as WAV `cue`/`LIST adtl`
  chunks.

**Meter (peak / RMS)**
- *Plain:* the bars that show how loud the sound is right now.
- Output meter: peak since the last frame + 300 ms sliding RMS of the post-rack signal
  (`vox_engine::telemetry::Meter`), sent in `VXTM` frames.

**Module / Module API**
- *Plain:* one effect in the rack, and the rulebook every effect follows.
- `vox_module_api::Module` (`activate`/`process`/`reset`/`deactivate`, parameter schema, events,
  state, extensions) — ADR-005. See [plugins.md](architecture/plugins.md#the-module-api).

**Monitoring (off / dry / through rack)**
- *Plain:* hearing yourself in headphones while recording, either plain or with effects.
- Input → monitor ring → drift-corrected resampler → output callback; "through rack" feeds the
  shared playback rack. Default off.

## N

**Noise print / noise reduction**
- *Plain:* you teach the app what the background hiss sounds like, and it removes that sound.
- A captured `NoiseProfile` blob (8192-point power spectrum, `PVNP` v1) driving a streaming
  spectral Wiener-gain reducer (`vox_dsp::nr`). See [dsp.md](architecture/dsp.md#noise-reduction).

**Normalize (peak / LUFS)**
- *Plain:* make the recording louder or quieter so it hits a target level.
- A destructive, undoable gain: peak normalize to a dBFS target, LUFS normalize to an integrated
  loudness target (`vox_project::normalize`).

## O

**Offline render**
- *Plain:* running the effects as fast as possible to produce a file, instead of in real time.
- `vox_rack::offline::render` / `render_range` — the same chain code as playback, in
  `ProcessMode::Offline`.

**Orchestrator**
- *Plain:* the lead AI session that plans the work and hands tickets to helper sessions.
- See [contributing.md](contributing.md#how-work-is-organised).

## P

**Peak pyramid**
- *Plain:* a pre-computed zoomed-out summary of the waveform so drawing is instant.
- Per-chunk min/max buckets at 64…65 536 samples per bucket (`vox_project::store::peaks`),
  served to the UI as binary `VXPK` frames.

**Piece table**
- *Plain:* the document is a list of "play this stretch of that chunk" entries; edits just
  rewrite the list.
- `Vec<Piece{source, offset, len}>` in a `DocSnapshot` (`vox_project::snapshot`).

**Placeholder (Missing / Unreadable slot)**
- *Plain:* a rack slot for an effect that isn't installed any more; its settings are kept.
- A slot the registry can't resolve; it passes audio through, reports latency 0 and is written
  back verbatim. It recovers live when the module appears again (H-40).

**Pre-roll / post-roll**
- *Plain:* a few seconds played before and after the part you re-record, so you can get into
  the flow.
- Punch-in pre-roll 5 s and post-roll 1 s by default. Separately, the *rack pre-roll* (H-46)
  warms the effects up before playback so the first heard sample is the play position.

**Preset**
- *Plain:* a saved set of settings for one effect or for the whole rack.
- JSON files under `<config>/presets/modules/<id>/` and `<config>/presets/racks/`
  (`crates/presets`), plus built-in factory presets.

**Punch-in**
- *Plain:* re-record just one part of your take, over a selection.
- A record operation that replaces exactly the selection, with pre-/post-roll and 10 ms
  equal-power crossfades (SPEC-022). See [runtime.md](architecture/runtime.md#record-and-punch-in).

## R

**Rack / slot**
- *Plain:* the chain of effects your voice goes through; each position in it is a slot.
- `vox_rack::Chain` of up to 16 slots in series, hosted live by `RackHost` (control thread) and
  `LiveRack` (audio thread).

**Real-time safe**
- *Plain:* code fast and predictable enough to run inside the audio callback without glitches.
- No allocation, locks, I/O, logging, syscalls or unbounded loops; checked in tests with
  `vox_module_api::test_util::no_alloc`.

**Reader (prefetch)**
- *Plain:* a helper that reads the next bit of audio from disk before it's needed.
- `vox-reader` keeps ~200 ms of resampled audio in the playback ring; the audio thread never
  touches the chunk store.

**Resampling**
- *Plain:* converting audio from one sample rate to another (e.g. 44.1 kHz → 48 kHz).
- `rubato` via `vox_dsp` (`resample`, `capture_resample`, `async_resample`).

**Return ring**
- *Plain:* a one-way queue that sends finished objects from the audio thread back to be deleted
  safely elsewhere.
- `LiveRack` pushes retired chains onto a 64-entry rtrb ring; `RackHost::tick` drops them
  (ADR-002).

**Revision (`rev`, `audio_rev`)**
- *Plain:* version numbers of the document, so the UI knows when to redraw.
- Monotonic per change; undo/redo produce fresh values, so compare with `!=`, never `<`.

**Ring buffer (SPSC)**
- *Plain:* a fixed-size circular queue with one writer and one reader, safe without locks.
- `rtrb` rings between the control, reader, input and output threads.

## S

**Sample rate / bit depth**
- *Plain:* how many sound snapshots per second, and how precise each one is.
- Default 48 kHz / 24-bit for new recordings; internal processing is 32-bit float.

**Sandbox**
- *Plain:* a separate helper program that runs each third-party plugin, so a crashing plugin
  can't crash PowerVoice.
- One `powervoice-sandbox` process per plugin instance, connected by shared memory + a JSON
  control pipe (ADR-008). See [plugins.md](architecture/plugins.md#the-sandbox).

**Scan (plugin scan)**
- *Plain:* PowerVoice looking through your plugin folders to find effects it can load.
- Each file is scanned in its own `powervoice-sandbox --scan` process with a 30 s timeout;
  results are cached in `plugin-scan.json`.

**Shadowed plugin**
- *Plain:* a second copy of a plugin that's ignored because another copy with the same id wins.
- Duplicate ids resolve by tier: modules folder → install folder → standard paths → custom
  folders (ADR-008 Amendment 5).

**Sidecar (`.vo.json`)**
- *Plain:* a small file saved next to your audio that remembers effects, markers and view.
- `<file>.vo.json` (e.g. `take.wav.vo.json`), schema v1 (`vox_project::sidecar`). See
  [data.md](architecture/data.md#the-sidecar).

**Snapshot**
- *Plain:* a frozen picture of the document at one moment; undo switches between snapshots.
- `Arc<DocSnapshot>` (piece table + markers + revs); never mutated once published.

**Spec**
- *Plain:* a document describing exactly how a feature must behave, with measurable tests.
- `specs/SPEC-NNN-*.md`.

**Spectrogram / spectral view**
- *Plain:* a picture of the sound where height is pitch, left-to-right is time and brightness is
  loudness.
- STFT tiles computed in Rust (`vox_engine::spectro`), sent as u8 `VXST` frames, coloured on the
  GPU. See [ui.md](architecture/ui.md#renderers).

**Spectrum analyzer**
- *Plain:* the live graph of which frequencies are in the sound you hear right now.
- Post-rack tap → BH4 FFT → 1/24-octave bands → `VXSA` frames at up to 60 Hz.

## T

**Take**
- *Plain:* one recording pass.
- A crash-safe 32-bit float WAV under `takes/` plus chunks in the store, committed as one edit
  (`vox_project::take`, `TakeCapture`).

**Telemetry**
- *Plain:* the live numbers the audio engine sends to the screen (playhead, meters).
- Binary frames on Tauri channels: `VXTM` (transport + meter), `VXMT` (module meters), `VXSA`
  (analyzer). See [ipc.md](architecture/ipc.md#binary-frames).

**Ticket**
- *Plain:* one unit of work, with a clear definition of done.
- `tickets/T-NNN-*.md` / `H-NN` / `S#-NN`, tracked in `tickets/BOARD.md`.

**Tile (spectrogram tile)**
- *Plain:* one small square of the spectrogram picture, computed separately so it can be cached.
- 256 STFT frames × bins, content-keyed LRU cache (`vox_engine::spectro`).

**True peak**
- *Plain:* the real loudest point of the sound, including peaks that fall between samples.
- Measured with 4× oversampling (`vox_dsp::true_peak`); the limiter keeps it under the ceiling.

**ts-rs**
- *Plain:* a tool that writes the UI's type definitions from the Rust code so both sides agree.
- `ts-rs` 12; `just gen-types` writes `ui/src/lib/ipc/bindings.ts`, `just check-types` fails if
  it's stale.

## U

**Undo / redo**
- *Plain:* step back and forward through your edits.
- Unbounded stacks of snapshots in `vox_project::history`, limited only by the disk budget.

## V

**VST3 / VST2**
- *Plain:* Steinberg's plugin formats; VST3 is supported, VST2 is not.
- VST3 via the `vst3` crate in the sandbox. VST2 is gated on a legal decision (T-811) and not
  implemented.

**`.voxmod`**
- *Plain:* a package file for installing a PowerVoice add-on effect.
- A validated zip: `manifest.json`, one CLAP binary per platform, licenses, SHA-256 checksums
  (`vox_plugin_host::voxmod`, ADR-006).

**`VXTM`, `VXMT`, `VXSA`, `VXST`, `VXPK`, `VXIS`, `VXLT`**
- *Plain:* compact binary messages for bulk data (meters, waveform, spectrogram).
- Little-endian frames with a 4-byte magic, version and header length. See
  [ipc.md](architecture/ipc.md#binary-frames).

## W

**Watchdog**
- *Plain:* a guard that notices when a plugin crashes or freezes and stops it.
- `sandbox-watchdog` thread in `vox_plugin_host` (10 ms poll), plus the host's bounded waits
  in the audio thread.

**Waveform / peaks**
- *Plain:* the drawing of the sound's shape over time.
- Drawn from `VXPK` min/max buckets (or raw samples when zoomed in).

**WebView**
- *Plain:* the built-in browser engine that draws PowerVoice's window.
- WebKitGTK on Linux, WebView2 on Windows, WKWebView on macOS (Tauri 2).

**Worktree**
- *Plain:* a separate copy of the code where one ticket is worked on without disturbing others.
- `git worktree` under `.claude/worktrees/T-NNN` on branch `ticket/T-NNN`.

## Z

**Zero crossing**
- *Plain:* a point where the waveform crosses silence; cutting there avoids clicks.
- Optional snap of selection edges (±512 samples, applied when the gesture ends; A-021).

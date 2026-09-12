# SPEC-000 — Architecture overview

- **Status:** draft
- **Milestone:** M1 (applies to every later milestone)
- **Related:** ADR-001 … ADR-009 · SPEC-001, SPEC-002, SPEC-003, SPEC-004,
  SPEC-012

## 1. Purpose
Specs describe **behavior**, ADRs describe **structure**. This page is the map a spec reader or
implementer needs before opening any other spec. It shows which part of the system produces each
user-visible behavior and where the structural decision is recorded. It also fixes the shared
vocabulary: every VoxEdit spec uses the terms in §2.4 with exactly these meanings.

## 2. Behavior / UX

### 2.1 The system in one paragraph
VoxEdit is one desktop process: Tauri 2, with a Rust core and a system WebView. The Svelte UI only
renders and sends intents. Audio, DSP, file I/O and document state all live in Rust (ADR-001 §1).
The engine runs two real-time device callbacks: output is always open, input is opened on demand.
They touch only lock-free rings, atomics, the rack and preallocated buffers (ADR-002). The open
document lives in a disk-backed **session store** as immutable **chunks**, addressed by immutable
**snapshots**. Undo is therefore a pointer swap, and RAM use is independent of file length and undo
depth (ADR-004). Every effect is a **module** behind one Module API (ADR-005). One **rack**
implementation serves playback, monitoring, export, bake, analysis and the CLI (ADR-001 §5). The UI
receives bulk data as binary IPC (ADR-003). In M8, external plugins run in one sandbox process each
(ADR-008), and installable modules are CLAP bundles (ADR-006). ADR-007 confines licensing-sensitive
code (LAME, GPL) to known edges.

### 2.2 Where each behavior comes from

| User-visible behavior | Spec | Structure | Crates | Tickets |
|---|---|---|---|---|
| Devices, input channel, hot-plug, device loss | SPEC-001 | ADR-002 §1, §7 | `engine`, `src-tauri` | T-102, T-104 |
| Recording, input meter, monitoring, dropouts | SPEC-002 | ADR-002 §1, §6–§8; ADR-004 §7 | `engine`, `project` | T-106, T-107, T-108 |
| Transport, playhead, rate mismatch | SPEC-003 | ADR-002 §4, §5, §8; ADR-003 §3 | `engine`, `ui` | T-105, T-108 |
| Document, undo, memory/disk budgets, recovery, cleanup | SPEC-004 | ADR-004 | `project`, `engine` | T-101 (M1), T-301 (M3) |
| Rack, bypass, parameters, latency, offline render, generic UI | SPEC-012 | ADR-005; ADR-001 §5 | `module-api`, `rack`, `modules`, `cli` | T-103 (M1); T-401, T-405, T-406 (M4) |
| Peaks, spectrogram tiles, telemetry transport | per-feature specs | ADR-003 | `src-tauri`, `ui` | T-108, M2 |
| Renderers, Linux WebView workarounds | M2 specs | ADR-009 | `ui` | T-007, M2 |
| External plugins, installable modules | M8 specs | ADR-006, ADR-007, ADR-008 | `plugin-host`, `plugin-sandbox` | M8 |

Spec numbers other specs already reserve: **SPEC-011** ACX check (M6), **SPEC-017** true-peak
limiter (M4) and **SPEC-019** full shortcut map (M7). The orchestrator assigns the rest in each
milestone's spec wave.

### 2.3 Signal paths (behavioral view of the ADR-002 §1 diagram)
- **Playback.** Session store → reader thread (resamples document → device rate only when the
  device can't run at the document rate) → playback ring → rack (realtime) → device. The playhead
  shows the **heard position** (§2.4).
- **Recording.** Device input → selected channel → capture ring → capture-writer → take WAV +
  session store. The rack and monitoring never touch the take, and neither can a plugin failure
  (ADR-008 §5).
- **Monitoring.** Device input → monitor ring (drift servo) → output. The input is added either after
  the rack (**dry**) or before it (**through rack**). Monitoring is never recorded.
- **Offline render.** A snapshot or WAV goes into a rack with its own instances, in offline mode, then
  to an encoder, bake, an analyzer or the CLI. Live playback continues meanwhile.
- **Control.** UI → command → control thread → lock-free queues → audio threads. The audio threads
  send RT events to the control thread, which turns them into events and 60 Hz telemetry for the UI.

### 2.4 Glossary

| Term | Meaning |
|---|---|
| **Document** | The one open mono audio file being edited: samples at one sample rate, plus markers. It is persisted as the plain audio file plus the sidecar `name.vo.json` (PROMPT §2), and edited through the session store. |
| **Document time** | A position counted in samples at the **document's** sample rate (`u64`, suffix `_samples`); 0 is the first sample. Selections, markers, edits, every position in IPC, and every AC that says "at position …" use document time. It never depends on a device. |
| **Device time** | Frames counted on **one device stream** at that stream's rate. The input and output streams each have their own clock. Device times of different streams are never compared directly, because cpal `StreamInstant`s are per stream (ADR-002 context). |
| **App clock** | Monotonic nanoseconds since process start (`APP_EPOCH`). Every stream maps its timestamps onto it inside its own callback, and the UI synchronises to it. It is the only way to relate input, output and UI events in time (ADR-002 §8, ADR-003 §3). |
| **Heard position** | The document position audible at the speaker at a given app-clock time: position entering the rack − rack latency − output latency. The playhead shows it (SPEC-003). |
| **Document rate / device rate** | The document rate is fixed when a document is created (recording) or imported. The device rate is the rate the device stream runs at. They are equal whenever the device supports the document rate. Otherwise the reader (playback) or the capture-writer (recording) resamples (ADR-002 §5). |
| **Session (store)** | Working storage for one open document, in `<app local data dir>/sessions/<id>/`: chunks, peaks, journal and takes (ADR-004 §1). It is **not** the user's file. It makes unlimited undo and crash recovery possible. |
| **Chunk** | Up to 65 536 f32 samples (256 KiB) in the session store, immutable once committed. It is the unit of peak caching, checksums, recovery granularity and memory eviction. Edits never modify a chunk. |
| **Piece / piece table** | The document as an ordered list of pieces, each referencing a sub-range of a chunk or a run of silence. Cut, copy and paste only rearrange pieces. |
| **Snapshot** | An immutable revision of the document (`DocSnapshot`: piece table, markers, rate, length), shared by reference. Every committed change creates a new one, and a reader keeps the snapshot it started with. `rev` changes on every change, `audio_rev` only when samples change. |
| **Edit** | A committed change that produces a new snapshot. It is recorded in the journal and is undoable unless SPEC-004 says otherwise. A **destructive (audio) edit** changes samples and stops playback. A **marker edit** doesn't. |
| **Undo floor** | The oldest state undo can return to: the imported file, or the empty new document. |
| **Journal** | Append-only, checksummed log of edits, undo/redo and takes in the session, fsynced before an edit reports success. It is replayed for crash recovery. |
| **Take** | One continuous recording, from Record to Stop. It is captured as a crash-safe 32-bit float WAV in the session and becomes **one** undoable edit on Stop. |
| **Xrun** | A device callback that was not serviced in time. An **output underrun** glitches what is heard; an **input overrun** loses captured audio. Xruns are counted and reported (ADR-002 §7). |
| **Dropout** | Input audio lost during recording because of an input xrun. SPEC-002 fills it with silence and marks it. |
| **Rack / chain** | The ordered list of slots applied non-destructively to playback, through-rack monitoring and export. `RackModel` is its plain-data description, saved in the sidecar. A chain is the set of live instances built from a `RackModel`. |
| **Slot** | One position in the rack. It holds a module instance plus host-owned bypass. A slot whose module is not installed is a **placeholder**: it passes audio dry and keeps its stored state. |
| **Module** | Anything that can sit in a slot, implementing the Module API (ADR-005). It is either built-in (compiled in), installed (CLAP bundle, ADR-006) or an external-plugin adapter (M8). |
| **Parameter value (plain / normalized)** | The **plain** value is the value in its unit (e.g. −6.0 dB). It is stored, sent to the module and shown as text. The **normalized** value is a 0–1 control position through the parameter's taper, used only by UI widgets. |
| **Smoothing** | The module-owned transition from a parameter's old value to its new one, over its declared `smoothing_ms`, so there are no clicks or zipper noise (SPEC-012 §4.3). |
| **Latency** | *Module:* the samples by which its output lags its input. *Rack:* the sum over all slots, bypassed slots included. *Monitoring:* the total delay from the microphone to the headphones (SPEC-002). |
| **Bypass** | A host-owned 15 ms crossfade to the latency-matched dry signal, for one slot (slot bypass) or for the whole rack (A/B). Bypassed modules keep processing. |
| **Realtime / offline render** | *Realtime:* the rack runs inside the output callback with device-sized blocks. *Offline:* the rack runs in a job (export, bake, analysis, CLI) with its own instances and 4096-frame blocks. Both use the same code and agree within 1e-6 (SPEC-012). |
| **Sub-block** | A slice of at most 1024 frames of a device callback (`MAX_BLOCK`), the unit the rack processes in realtime. |
| **Epoch** | A counter tagging playback packets, so that stale audio is discarded after a seek, stop or loop wrap. |
| **Telemetry frame** | The 60 Hz (30 Hz selectable) binary message carrying the playhead anchor, meter values and xrun/clip flags (`VXTM`, ADR-003). |
| **Fake backend** | The engine audio backend without hardware. It drives the *real* callback code with synthetic timing, random buffer sizes and injected faults, and is the basis of the integration tests. |
| **testkit / `voxedit-cli analyze`** | The independent measuring stick: peak, RMS (`20·log10(rms)`), LUFS, true peak and noise floor. `-inf` means digital silence, `NaN` means non-finite input, and `None`/`n/a` means the input is shorter than the measurement window. JSON prints `null` for `-inf` and `NaN`. |

### 2.5 Naming note
PROMPT §3.7's `prepare(sample_rate, max_block)` and `tail_samples()` are `activate(&ActivateConfig)`
and `tail()` in ADR-005 (decision D-008). Specs use the ADR-005 names.

## 3. Parameters
These are system constants that behavior specs rely on. The ADR named in "Source" owns each value; a
change is made there first.

| id | name | unit | value | source |
|---|---|---|---|---|
| `MAX_BLOCK` | Realtime rack sub-block | frames | 1024 | ADR-002 §2 |
| `OFFLINE_BLOCK` | Offline render block | frames | 4096 | ADR-002 §2 |
| `CHUNK_SAMPLES` | Chunk size | samples | 65 536 | ADR-004 §2 |
| `CONTROL_TICK` | Control-thread tick | ms | 16 | ADR-002 §1 |
| `TELEMETRY_RATE` | Playhead/meter frames | Hz | 60 (30 selectable in Settings) | ADR-003 §1, ADR-009 §3 |
| `DEVICE_POLL` | Device enumeration interval | s | ~1 | ADR-002 §1 |
| `PREFETCH` | Playback ring fill | ms | ~200 | ADR-002 §5 |
| `CAPTURE_RING` | Capture ring | s | 10 | ADR-002 §2 |
| `EVENT_CAPACITY` | Parameter events per slot per block | events | 512 | ADR-002 §3 |
| `XFADE` | Bypass / chain-swap crossfade | ms | 15 (T-103 may tune within 10–20) | ADR-005 §9 |
| `START_LATENCY` | Play → first audible sample | ms | < 50 | PROMPT §4, ADR-002 §5 |
| `MEM_BUDGET` | Resident audio memory | bytes | clamp(RAM/4, 512 MiB, 4 GiB) | ADR-004 §4 |
| `DISK_SOFT_LIMIT` | Session disk use | bytes | max(8 GiB, 8 × document bytes), keep ≥ 2 GiB free | ADR-004 §4 |
| `RT_OFFLINE_TOL` | Realtime vs offline render difference | abs | ≤ 1e-6 (≈ −120 dBFS) | ADR-001 §5 |

## 4. Algorithm / implementation notes
- **Precedence.** A behavior spec never restates structure; it cites the ADR. When a spec needs a
  structural change, it says so explicitly and the orchestrator amends the ADR. Conflicts are
  reported, never silently resolved.
- **Testable architecture rules.** The ACs below turn the parts of ADR-001/002/003 that apply to
  every feature into system-level checks. Feature specs refine them and do not repeat them.
- **Cross-document notes found while writing the M1 spec wave (T-008).** _Status at merge
  (orchestrator): notes 1–4 and 6 are resolved (ADR-002, ADR-004 and CLAUDE.md amended; step-derived
  decimals implemented by T-005). Note 5 waits for the owner's SPEC-004 OD-4 decision._ These were reported to the
  orchestrator, not resolved here:
  1. CLAUDE.md says to drop heavy objects off the audio thread with "`basedrop` or a worker thread".
     ADR-002 rejects `basedrop` in favour of the return ring. CLAUDE.md should say "the return ring
     (ADR-002)".
  2. ADR-002 §2 says a capture-ring overflow "marks the take damaged". ADR-004 §7.4 says it
     "finalizes the take at the last good sample". SPEC-002 §2.4 follows ADR-004 (stop and keep).
  3. ADR-002 §1 opens the input stream when "monitoring ≠ off". SPEC-002 refines this: monitoring is
     audible only while the input is **armed** or recording, so a persisted monitoring preference
     never opens the microphone at start-up.
  4. ADR-004 §6 promises a recording "loses nothing on a process crash". Audio still in the capture
     ring or in the writer's hands when the process dies is necessarily lost. SPEC-002 AC-6 puts a
     number on it (≤ 250 ms).
  5. ADR-004 undo entries are bare `DocSnapshot`s. SPEC-004's recommended "undoing a bake restores
     the pre-bake rack" needs an optional opaque rack-state attachment per undo entry and per journal
     `edit` record. `project` must treat it as opaque data, because ADR-001 forbids `project → rack`.
     This needs an ADR-004 amendment if the owner accepts the SPEC-004 OD-4 default.
  6. The T-005 `module-api` formats stepped parameters with the declared `decimals`. SPEC-012 §4.2
     derives them from the step so text round-trips exactly. That is a code change for T-103.

## 5. Acceptance criteria
- **AC-1 (crate edges).** Given the workspace, when `cargo tree -e normal -p <crate>` is run for
  every library crate, then no forbidden edge from ADR-001 §3 rule 4 appears. In addition, `cpal`
  appears only under `vox-engine`, `memmap2` only under `vox-project`, `tauri`/`ts-rs` only under
  `voxedit-app`, and `rubato`/`realfft` only under `vox-dsp`.
- **AC-2 (real-time safety, whole engine).** Given the fake backend with random callback sizes
  1…4096 frames, when a scripted 60 s simulated session runs play, seek, loop wrap, record, all three
  monitoring modes, 20 rack edits and 200 parameter changes, then `assert_no_alloc` reports
  **0** allocations and **0** deallocations inside the input/output callbacks and `process()`.
- **AC-3 (one rack code path).** Given the same input WAV and the same rack description, when
  rendered by `voxedit-cli render --rack` and by the engine's offline render job, then the outputs
  are bit-identical (FNV-1a hash equal). The realtime path matches them within 1e-6 absolute
  (SPEC-012 AC-9).
- **AC-4 (document time vs device time).** Given a 44 100 Hz document played on a 48 000 Hz output
  device (fake backend, empty rack, output latency 0), when the output callback has consumed exactly
  48 000 frames past the start of audible output, then the reported heard position has advanced by
  44 100 ± 1 document samples.
- **AC-5 (device-free testability).** Given a machine with no audio devices and no display
  (`WAYLAND_DISPLAY` and `DISPLAY` unset), when `cargo test -p vox-dsp -p vox-module-api -p vox-rack
  -p vox-modules -p vox-testkit` runs, then every test passes. No test opens a device or a WebView,
  and none writes outside a temporary directory.
- **AC-6 (cross-platform build).** Given the Windows GNU target installed (MEMORY environment), when
  `just check-cross` runs, then every crate except `src-tauri` type-checks with 0 errors and 0
  warnings.
- **AC-7 (binary IPC contract).** Given the golden `VXPK`, `VXST`, `VXTM` and `VXRP` frames produced
  by the Rust `gen_ipc_fixtures` test, when Vitest decodes them with `ui/src/lib/ipc/binary.ts`,
  then every header field equals its known value and every payload float is bit-identical. No IPC
  command or event carries a JSON array of audio samples or peaks.

## 6. Test plan

| AC | Kind | Where / fixtures |
|---|---|---|
| AC-1 | integration (script) | proposed `just deps-check` recipe (ADR-001 follow-up), run inside `just check` |
| AC-2 | integration (fake backend) | `engine` test harness (T-105), seeded script; runs in `just test` |
| AC-3 | integration | `cli` test: `voxedit-cli gen pink` fixture → `render --rack` vs engine job; T-103/T-602 |
| AC-4 | integration (fake backend) | `engine` (T-105); fake device rate 48 000 Hz, document 44 100 Hz |
| AC-5 | integration (environment) | `just test` under `env -u WAYLAND_DISPLAY -u DISPLAY`; no fixture |
| AC-6 | integration (build) | `just check-cross` |
| AC-7 | unit (Rust) + unit (Vitest) | `ui/src/lib/ipc/__fixtures__/` (ADR-003 §4) |

## 7. Out of scope
- Structural decisions themselves (the ADRs) and the renderer choice (ADR-009, T-007).
- Feature behavior: see the per-feature specs in §2.2.
- Performance targets other than start latency; those belong to T-704 and the feature specs.

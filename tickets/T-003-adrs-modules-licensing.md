# T-003 — ADRs B: Module API, installable-module ABI, licensing, sandbox outline

- **Milestone / wave:** M0 / W1
- **Tier:** Opus
- **Depends on:** —
- **PROMPT refs:** §2, §3.4, §3.7 · **References:** `docs/references.md`, `MEMORY.md`

## Goal
ADR-005, ADR-006, ADR-007, ADR-008 in `docs/adr/` (format: `docs/adr/README.md`). Docs only. ADR-005 must be precise enough that T-005 can implement it directly (include Rust trait/struct sketches).

## Decisions already made (record, justify, detail)
**ADR-005 Module API** (designed so M8 plugin adapters and CLAP-packaged modules fit without a rewrite)
- Descriptor: reverse-DNS `id`, semver `version`, name, vendor, features/categories. Presets and sidecar reference `id@version`.
- Parameters: stable `u32` id + stable string key (sidecar stores keys); **plain values** with range, taper (linear/log/dB), step, enum labels, default, unit; flags: automatable, stepped, bool, read-only/output, hidden, is-bypass; `value_to_text` / `text_to_value`.
- Parameter changes arrive as a **per-block event list with sample offsets** (CLAP-like). The module owns smoothing.
- Lifecycle: `activate(sample_rate, max_block, ProcessMode { Realtime, Offline })` — off audio thread, may allocate; `process()` — any block length ≤ max_block including 0, RT-safe; `reset()` — RT-safe, on seek/loop wrap; `deactivate()`. Sample-rate change = deactivate → activate → swap via the same path as rack edits.
- `latency_samples()`, `tail()` (`Samples(n)` | `Infinite`) valid after activate; module can raise `latency_changed` → host re-activates off-thread and recomputes compensation.
- Channel layout: `process(&[&[f32]], &mut [&mut [f32]])` + `supported_layouts()`; v1 negotiates only 1-in/1-out; the dual-mono shim for stereo-only plugins lives once in the host.
- **Bypass is host-owned**: latency-matched dry path + 10–20 ms crossfade; whole-rack A/B delays dry by total rack latency. If a module declares an is-bypass param, the host maps to it.
- State: `{ format_version, params: key→value, blob: Option<bytes> }` + migrate hook; external plugins are blob-only.
- Optional extension traits (queried at runtime): `Telemetry` (lock-free readouts, e.g. gain reduction), `ResponseCurve` (EQ graph from the same coefficients), `NoiseProfile` (capture/store profile).
- Threading: modules are `Send`, not `Sync`; control-thread calls never run concurrently with `process`.
- How the generic parameter UI is derived from the schema (what the UI needs: grouping, display order).

**ADR-006 Installable module ABI**
- Separately installed modules are **CLAP bundles** built with `clack-plugin` (pure Rust, MIT/Apache) — no custom ABI; avoid nih-plug's GPLv3 VST3 path. Our extra metadata (parameter grouping, preset packs, ResponseCurve/Telemetry equivalents) — define how it travels (CLAP extensions/custom extension id or sidecar JSON in the bundle).
- Built-in modules stay compiled-in in v1 but implement the same Module API; M8 T-805 proves one built-in module as a CLAP package.
- Install location and "Install module…" flow (copy into app plugin dir; scan).

**ADR-007 Licensing**
- Table: every planned dependency (see `docs/references.md` ecosystem facts) with license, linking mode, obligations.
- LAME (LGPL-3.0): **dynamic linking** to libmp3lame; how that works with `mp3lame-encoder` (check whether it supports dynamic linking; if not, propose the alternative) → **flag for owner confirmation before M6**.
- symphonia MPL-2.0 (file-level copyleft) obligations.
- ysfx (GPLv3): linked only into the `plugin-sandbox` binary (separate process, IPC boundary); note that the process-boundary argument should be reviewed before public distribution.
- VST3 SDK MIT since 3.8.0 (Oct 2025) — record version to use.
- VST2: reverse-engineered headers only, DMCA precedent → gated (T-811), prefer Carla bridge.
- Project license: not chosen by owner — list the implications of MIT vs GPL-3.0 given the above, as an open question.

**ADR-008 Plugin sandbox outline** (high level; M8 details it)
- One sandbox process per plugin vs per chain (recommend with rationale), shared-memory audio ring + control channel, wakeup primitives per OS (eventfd/futex on Linux, named events on Windows, semaphores on macOS), watchdog, crash → silence/bypass + user notice + flag, added latency reported via `latency_samples()`.
- Scanning happens inside the sandbox. Floating editor windows owned by the sandbox (Wayland has no embedding; XWayland/X11).
- Prior art: carla-bridge, yabridge, Bitwig, `shmem-ipc`, `iceoryx2`.

## Scope
**In:** four ADR files. **Out:** code, index update (orchestrator), specs.

## Acceptance
- [ ] ADR-005 contains Rust sketches of: `ModuleDescriptor`, `ParamInfo`, `ParamFlags`, `Taper`, `ParamEvent`, `ProcessContext`, `ProcessMode`, `Tail`, `ModuleState`, `trait Module`, extension traits and the extension query mechanism.
- [ ] Consistent with PROMPT §2 LOCKED decisions.
- [ ] Owner-facing open questions listed explicitly (LAME linking, project license).

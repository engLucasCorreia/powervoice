# Architecture Decision Records

Each ADR records one decision, its context and its consequences. Decisions are **not rewritten**
when reality moves: an **Amendment** section is appended, and the last amendment wins. Where an
ADR and the code disagree, the code is the reference and the difference is listed in
[architecture/overview.md](../architecture/overview.md#where-the-code-differs-from-the-adrs).

Format for each `ADR-NNN-title.md`:

```
# ADR-NNN — Title
- Status: proposed | accepted | superseded by ADR-…
- Date: YYYY-MM-DD
- Deciders: owner, orchestrator

## Context
## Decision
## Consequences (positive / negative / follow-ups)
## Alternatives considered
## Open questions
```

## Index

| ADR | Title | Status | Amendments | Read it with |
|---|---|---|---|---|
| [ADR-001](ADR-001-architecture-overview.md) | Architecture overview & crate graph | accepted (M0, 2026-09-12) | — | [overview.md](../architecture/overview.md) |
| [ADR-002](ADR-002-threading-realtime.md) | Threading & real-time rules | accepted (M0) | 1 monitoring as implemented (T-107) · 2 the sandbox transport's syscall exception (T-801) · 3 loop playback without a rack reset (H-37) · 4 rack pre-roll on start (H-46) | [runtime.md](../architecture/runtime.md) |
| [ADR-003](ADR-003-ipc-data-paths.md) | IPC data paths & shared types (ts-rs 12) | accepted (M0) | 1 waveform/spectral frames (T-200) · 2 record events (T-300) · 3 slices + hardening frames and job kinds · 4 the live analyzer `VXSA` (T-208, supersedes 1's table) · 5 record operations + telemetry rate (T-304, H-16) | [ipc.md](../architecture/ipc.md) |
| [ADR-004](ADR-004-document-storage.md) | Document storage, snapshots, undo, journal | accepted (M0) | 1 bake undo carries the rack · 2 markers & sidecar (T-300) · 3 history labels, recovery, disk budget (T-301) · 4 punch-in journal records · 5 T-101 implementation notes · 6 record operations (T-304) · 7 bake attachment (T-602) | [data.md](../architecture/data.md) |
| [ADR-005](ADR-005-module-api.md) | Module API | accepted (M0) | 1 slices + hardening (telemetry, `set_param_plain`) · 2 open question 1 closed · 3 `AdapterHealth` + restart policy (T-802) · 4 asynchronous factories, `ParamText`, loading slots (T-803) · 5 `PluginEditor` (T-901) | [plugins.md](../architecture/plugins.md) |
| [ADR-006](ADR-006-installable-module-abi.md) | Installable module ABI (CLAP bundles, `.voxmod`) | accepted (M0) | 1 "Install module…" for a bare `.clap` (T-809) · 2 packaged modules as implemented (T-805) | [plugins.md](../architecture/plugins.md#installable-modules-voxmod) |
| [ADR-007](ADR-007-licensing.md) | Dependency & plugin-format licensing | accepted (M0) | owner decisions (M0) · T-200 codecs · CLAP bindings hand-written (T-803) · VST3 SDK version (T-806) · LV2 headers + lilv at run time (T-807) · vendored ysfx (T-808) · clack + `zip` for `.voxmod` (T-805) · libX11 and suil at run time (T-901) | [THIRD_PARTY_NOTICES](../../THIRD_PARTY_NOTICES) |
| [ADR-008](ADR-008-plugin-sandbox-outline.md) | Plugin sandbox outline | accepted (M0) | 1 transport (T-801) · 2 process, control channel, watchdog, proxy (T-802) · 3 CLAP backend + async loading (T-803) · 4 scanner, cache, blocklist (T-804) · 5 duplicate-id policy + uninstall (H-29) · 6 VST3 backend (T-806) · 7 plugin polish (H-34) · 8 LV2 backend (T-807) · 9 events travel with their chunk (H-36) · 10 live recovery of Missing slots (H-40) · 11 JSFX backend (T-808) · 12 PowerVoice modules in the CLAP backend (T-805) · 13 editor windows (T-901) | [plugins.md](../architecture/plugins.md#the-sandbox) |
| [ADR-009](ADR-009-renderer-choice.md) | Renderer choice & Linux workarounds | accepted (M0) | 1 DMA-BUF workaround on by default | [ui.md](../architecture/ui.md#renderers) |

Every ADR is `accepted`; none has been superseded. Product-level decisions that never needed an
ADR (owner decisions D-001…D-022 and autonomous decisions A-001…A-027) are logged in
[MEMORY.md](../../MEMORY.md).

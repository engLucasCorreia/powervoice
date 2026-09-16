# PowerVoice documentation

Every document in this repository, what it's for and who it's for. PowerVoice is a desktop editor
for voice-over: record, edit, clean with a non-destructive effects rack, hit loudness targets,
export. Start with the [README](../README.md) for what it does and how to install it.

## Start here

| Document | Purpose | Audience |
|---|---|---|
| [what-is-powervoice.md](what-is-powervoice.md) | What PowerVoice is, who it's for, how it compares to Adobe Audition, and what it deliberately doesn't do | Everyone, especially new/non-technical users |
| [how-it-works.md](how-it-works.md) | How sound travels through PowerVoice, why your recording is never damaged, and why plugins run sandboxed — explained without code | Everyone |
| [user-guide.md](user-guide.md) | How to use the app: first recording, punch-in, the rack, loudness and ACX, export, troubleshooting | Users |
| [faq.md](faq.md) | Common questions and problems, answered in plain words | Users |
| [shortcuts.md](shortcuts.md) | Every keyboard shortcut (generated from the shortcut registry) | Users |
| [glossary.md](glossary.md) | Every term, plain-language line first, then the precise meaning | Everyone |
| [building.md](building.md) | Building from source on Linux, Windows and macOS, and the optional libraries | Users building it themselves, developers |

## Architecture

| Document | Purpose | Audience |
|---|---|---|
| [architecture/overview.md](architecture/overview.md) | The map: context and container diagrams, the generated crate graph, what each crate does | Developers |
| [architecture/runtime.md](architecture/runtime.md) | Threads, the real-time rules, and a sequence diagram for every main flow (start, open, play, record, edit, bake, export, recovery, plugins) | Developers |
| [architecture/data.md](architecture/data.md) | Session storage (chunks, snapshots, journal), the sidecar, recovery, settings, presets, caches, where files live per OS | Developers |
| [architecture/ipc.md](architecture/ipc.md) | Commands, events, binary channels, ts-rs types; the generated command and event tables | Developers |
| [architecture/dsp.md](architecture/dsp.md) | The live and offline signal chains, latency, and every built-in module with its parameters | Developers, DSP contributors |
| [architecture/plugins.md](architecture/plugins.md) | The Module API, installable `.voxmod` modules, the sandbox, format backends, the scanner, and how to write a module | Plugin and module authors, developers |
| [architecture/ui.md](architecture/ui.md) | Component tree, stores, renderers, layout, design system, i18n, dev pages | UI developers |

## Decisions and specifications

| Document | Purpose | Audience |
|---|---|---|
| [adr/README.md](adr/README.md) | Index of every architecture decision with its status and amendments | Developers |
| [adr/ADR-001-architecture-overview.md](adr/ADR-001-architecture-overview.md) | Crate graph and dependency rules | Developers |
| [adr/ADR-002-threading-realtime.md](adr/ADR-002-threading-realtime.md) | Threading model and real-time rules | Developers |
| [adr/ADR-003-ipc-data-paths.md](adr/ADR-003-ipc-data-paths.md) | IPC data paths and shared types | Developers |
| [adr/ADR-004-document-storage.md](adr/ADR-004-document-storage.md) | Document storage, snapshots, undo, journal | Developers |
| [adr/ADR-005-module-api.md](adr/ADR-005-module-api.md) | The Module API contract | Module authors |
| [adr/ADR-006-installable-module-abi.md](adr/ADR-006-installable-module-abi.md) | Installable module ABI (CLAP, `.voxmod`) | Module authors |
| [adr/ADR-007-licensing.md](adr/ADR-007-licensing.md) | Dependency and plugin-format licensing | Developers, packagers |
| [adr/ADR-008-plugin-sandbox-outline.md](adr/ADR-008-plugin-sandbox-outline.md) | Plugin sandbox design | Developers |
| [adr/ADR-009-renderer-choice.md](adr/ADR-009-renderer-choice.md) | Renderer choice and Linux workarounds | UI developers |
| [`specs/`](../specs/) | The behaviour specifications (SPEC-000…SPEC-022): UX, parameters, acceptance criteria with tolerances | Developers, testers |
| [references.md](references.md) | Standards and reference numbers used when writing specs (BS.1770, ACX, filters, dither) | Spec writers |

## Development

| Document | Purpose | Audience |
|---|---|---|
| [contributing.md](contributing.md) | Dev setup, `just` recipes, how the tests are organised, the ticket workflow, and recipes for adding a module, setting, command or i18n key | Contributors |
| [performance.md](performance.md) | The performance targets and the last measured numbers, with how to reproduce them | Developers |
| [design/design-system.md](design/design-system.md) | Tokens, components, themes, motion, accessibility, panel and dialog anatomy | UI developers, designers |
| [design/ui-audit.md](design/ui-audit.md) | The UI audit that produced the design system | UI developers, designers |
| [checkpoints/M0.md](checkpoints/M0.md) | The M0 foundations checkpoint (historical) | Project history |
| [checkpoints/M2-smoke.md](checkpoints/M2-smoke.md) | Manual smoke checklist for the editor view | Testers |

## Project files outside `docs/`

| File | Purpose |
|---|---|
| [PROMPT.md](../PROMPT.md) | The founding brief: mission, locked product decisions, scope, milestones |
| [MEMORY.md](../MEMORY.md) | Curated project memory: decisions, conventions, gotchas, per-ticket learnings |
| [CLAUDE.md](../CLAUDE.md) | Rules for AI agents working on the project |
| [tickets/BOARD.md](../tickets/BOARD.md) | Ticket status board |
| [THIRD_PARTY_NOTICES](../THIRD_PARTY_NOTICES) | Third-party licences (generated) |

## Keeping the docs true

- The crate graph in `architecture/overview.md` and the command and event tables in
  `architecture/ipc.md` are **generated** from `cargo metadata` and `src-tauri/src/ipc/` by
  `just docs` (`scripts/docs/check.py --write`).
- `just check` fails if a generated section is stale, if a relative link or anchor doesn't
  resolve, if a Mermaid diagram looks unparseable, or if a page under `docs/` isn't linked from
  this hub.
- Diagrams and claims are written against the code, with file paths. Where an ADR or spec
  disagrees with the code, the code is documented and the difference is called out.

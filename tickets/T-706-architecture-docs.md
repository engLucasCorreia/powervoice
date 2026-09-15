# T-706 — Complete architecture and developer documentation (owner request)

- **Tier:** Opus (writing plus accuracy; no review loop, blocking findings only)
- **Owner request (2026-09-15):** "complete documentation, with all markdown files necessary with all the architecture and information about the app with mermaid graphs and everything so everybody understand this app even for the non technical people." This ticket covers the technical half; T-707 covers the non-technical half. Both must be done before the project is finished.
- **Depends on:** T-809 (plugin features settled). Run late. If a later ticket changes the architecture, its report must say so, and the orchestrator re-runs the affected sections before the project closes.
- **Read first:**
  - CLAUDE.md, PROMPT.md, MEMORY.md;
  - every ADR in `docs/adr/` with its amendments;
  - `specs/`;
  - the crate `lib.rs` docs;
  - `docs/building.md`, `docs/design/*`, `README.md`.

## Deliverables (Markdown in `docs/`, GitHub-renderable Mermaid diagrams)
1. **`docs/README.md`: the documentation hub.** One map of every document with a one-line purpose and its audience (everyone, users, developers, plugin authors).
2. **`docs/architecture/overview.md`:**
   - a C4-style context diagram and a container diagram (Tauri shell, Svelte UI, Rust core crates, the audio engine thread, the sandbox processes, the file system);
   - the crate dependency graph, generated from `cargo metadata` by a small script so it stays true;
   - the role of each crate in one paragraph.
3. **`docs/architecture/runtime.md`:**
   - the threading model (UI thread, control thread, audio callback, reader/prefetch, job workers, spectro workers, sandbox), with a diagram and the real-time rules explained;
   - sequence diagrams for: app start, open file, record (including punch-in), play, edit plus undo, apply the rack or bake, noise-print capture, loudness analysis and normalize, export, crash recovery, plugin scan, loading a sandboxed plugin, a plugin crash and restart.
4. **`docs/architecture/data.md`:**
   - the document/session storage model: chunk store, snapshots, journal, sidecar, recovery (ADR-004);
   - the Settings file, presets and caches (the scan cache, blocklist, health), with a diagram of where each file lives per OS.
5. **`docs/architecture/ipc.md`:**
   - commands, events and binary channels, with the ts-rs type-generation flow;
   - a table of every command and event, generated or checked against `ipc/mod.rs` by a script.
6. **`docs/architecture/dsp.md`:** the signal chain from input to meters and output, and every module (gate, noise reduction, EQ, dynamics, limiter, normalize, LUFS/true-peak measurement). Each gets a block diagram, its parameters with units, and a reference to its spec.
7. **`docs/architecture/plugins.md`:** the module API, the installable module ABI, the sandbox and IPC ring, the CLAP adapter, the scanner and blocklist, and how to write a PowerVoice module (a walkthrough using the in-repo test CLAP).
8. **`docs/architecture/ui.md`:**
   - the UI component tree and stores;
   - the design system and themes (link to `docs/design`);
   - i18n, the renderers (WebGL2/Canvas2D), the layout and splitters, the dev pages (`?gallery`, `?preview`).
9. **`docs/contributing.md`:**
   - dev setup, `just` recipes, tests (unit, golden, big, bench), the worktree/ticket workflow, how the orchestrator works, the commit conventions and the review rules;
   - how to add a module, a setting (the test factories from H-18), a command or an i18n key.
10. **`docs/adr/README.md`:** an up-to-date index with each ADR's status and its amendments.
11. **Glossary:** `docs/glossary.md`, shared with T-707. Technical terms get a plain-language line first, then the precise definition.

## Quality bar
- Every diagram is correct against the code as it is today. Cite file paths so readers can follow.
- No stale claims. Where the code and a spec disagree, document the code and list the gap in your report.
- Add a `scripts/docs/check.py`, run from `just check`:
  - relative links and anchors resolve;
  - Mermaid fences parse;
  - the crate graph and the command table are up to date, with a `--write` mode to regenerate them.
- No new dependencies. Python standard library only.

`just check` must pass.

# HANDOFF — picking PowerVoice up

For whoever (or whatever) continues this work, including a future session of me. Read this first,
then `CLAUDE.md` for the rules, `MEMORY.md` for what was learned, and `tickets/BOARD.md` for state.

## What PowerVoice is
A focused desktop editor for voice-over — record, edit, clean up, hit a loudness target, export —
built as a Tauri 2 app with a Rust core and a Svelte 5 interface. It is an alternative to Adobe
Audition's Waveform Editor, not a multitrack DAW. Mono, one recording at a time, on purpose.

## Where things stand (2026-09-21)
- **196 of 198 tickets done.** Open: **H-95** (verification of the Explain My Voice feature) and
  **H-103** (the suspected cause of an export freeze; needs a real reproduction first).
- **v0.3.2 is the public release.** Main is ~30 commits ahead, including the Explain My Voice
  feature and the export-hang fix. **Cut v0.4.0 once H-95 reports.**
- `just check` is green: ~4,650 tests — roughly 2,670 in the UI and 1,980 in Rust.

## Releasing — the owner tests first
**Never tag a release until the owner has personally tested the fixes in it** (their instruction,
2026-09-21). Merge and push main as usual; then build a local AppImage, give them a checklist of what
to try per fix, and wait for their confirmation before bumping the version and tagging. CI going
green and my own screenshots have both been wrong before.

## How the work is organised
- **Spec-first.** `specs/SPEC-0xx` define behaviour; `docs/adr/` records architecture decisions.
  Both are amended, never silently contradicted — append a dated amendment and say what it
  supersedes.
- **One ticket per unit of work** in `tickets/`, tracked in `tickets/BOARD.md`. An orchestrator
  session dispatches each to a subagent working in its own git worktree, then squash-merges.
- After every merge: `just check`, update the board and `MEMORY.md`, `just roadmap`, republish the
  dashboard artifact, push main.

## The things that actually bite
1. **`just check` passing means very little for anything visual.** It was green for an AppImage
   whose window was empty, a modal that truncated its own sentences, and a graph with no frequency
   axis. Screenshot the contents and look at them.
2. **Verify an agent's completion claim against the code before merging.** Several were wrong in
   ways the report did not admit — a benchmark that was never registered to run, a feature reported
   complete that did not exist.
3. **Never bundle a library that must match the user's system** — `libpipewire` and
   `libwayland-client` each shipped a broken AppImage before `check_bundle.py` started failing the
   build over them.
4. **Measure the DOM instead of doing rem arithmetic**: the root font is 13px, so `11rem` is 143px.
   That one cost three attempts at a layout fix.
5. **Subscribe to a job's events before starting it**, or a fast job's terminal event is lost and
   the UI hangs on "running" forever.

## Running it
```sh
npm ci --prefix ui
just dev      # the app in development
just check    # the full suite (must pass before anything merges)
just build    # release build + Linux bundles
```
AppImage bundling needs a Debian-like host or CI; see `docs/building.md` for the workarounds on a
rolling-release distribution.

## Where to read next
- `docs/README.md` — the documentation hub, including a non-technical introduction.
- `docs/architecture/overview.md` — how the pieces fit, with diagrams.
- `PROMPT.md` §2 — the locked decisions that constrain everything.

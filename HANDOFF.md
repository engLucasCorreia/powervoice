# HANDOFF — picking PowerVoice up

For whoever (or whatever) continues this work, including a future session of me. Read this first,
then `CLAUDE.md` for the rules, `MEMORY.md` for what was learned, and `tickets/BOARD.md` for state.

## What PowerVoice is
A focused desktop editor for voice-over — record, edit, clean up, hit a loudness target, export —
built as a Tauri 2 app with a Rust core and a Svelte 5 interface. It is an alternative to Adobe
Audition's Waveform Editor, not a multitrack DAW. Mono, one recording at a time, on purpose.

## Where things stand (2026-09-22 — development paused here)
- **204 of 207 tickets done** (1 dropped: VST2, T-811). Open: **H-103** (a rare freeze where the app stops responding and
  cannot be closed — needs a live reproduction on the owner's machine, see below) and **H-126**
  (a documentation sweep, dispatched as the pause began; check its branch `ticket/H-126`).
- **v0.4.1 is the public release** (2026-09-22), and main is level with it plus H-125. It fixed
  everything the owner reported against v0.4.0; they tested each fix in a local build before it
  was tagged.
- `just check` is green: ~4,870 tests — 2,868 in the UI and ~2,002 in Rust. It takes 10–15 minutes.
- GitHub CI on main is green (it had been failing on and off for two days; H-125 found why).

## To pick the work back up
1. Read this file, then `CLAUDE.md`, `MEMORY.md`, `tickets/BOARD.md` (the two `todo` rows).
2. `npm ci --prefix ui && just check` — expect the first build to take a while, the debug build
   cache was deleted to reclaim disk (it was 243 GB).
3. The next release needs the owner's testing first (see below). H-125's fix (a plugin installed
   while a project is opening no longer stays "missing") is on main and has never shipped.

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
6. **The shipped app is not the dev server.** Two fixes passed every screenshot and still failed for
   the owner (2026-09-22). `vite build` minifies theme colours to `#rrggbbaa`/`#rgb`, which the
   WebGL colour parser did not read — every selection was an opaque gray block in the release while
   the dev server looked perfect. Verify rendering in **WebKitGTK** (what Tauri uses on Linux), not
   Chromium: PyGObject `Gtk.OffscreenWindow` + `WebKit2 4.1`, `WEBKIT_DMABUF_RENDERER_FORCE_SHM=1`,
   `HardwareAccelerationPolicy.ALWAYS`; inject the minified tokens to mimic a release build.
7. **A Svelte 5 `$effect` tracks state read synchronously inside an async function it calls, up to
   the first `await`.** The recording waveform's poll read the 60 Hz record head there, so every
   frame restarted the effect and its cleanup threw away the reply in flight — the waveform stayed
   empty while recording, for as long as recording had existed. Wrap such reads in `untrack`.
8. **Instant IPC mocks hide timing bugs.** Mock live/streaming commands with a round trip slower
   than one frame (30 ms), or the test proves nothing about the real app.
9. **Every blocking wait on a child process needs a deadline** (H-119/H-120), or one stalled child
   takes the whole gate to its ceiling with nothing in the log saying why.
10. **Check GitHub CI after pushing** (`gh run list --workflow CI --limit 3`). A test failed there
   on and off for two days while every local run passed; the cause was a test engine running a
   never-advancing fake device on the real clock, so its stall detector killed the output after
   500 ms.

## Running it
```sh
npm ci --prefix ui
just dev      # the app in development
just check    # the full suite (must pass before anything merges)
just build    # release build + Linux bundles
```
AppImage bundling needs a Debian-like host or CI; see `docs/building.md` for the workarounds on a
rolling-release distribution. **Build test builds with `just build`, never a raw
`npm run tauri build`** — the raw command skips the sandbox binary and the denylist strip, and
produces exactly the broken AppImage that v0.3.0 and v0.3.1 shipped.

A downloaded AppImage has no run permission: `chmod +x` it, or double-clicking does nothing.

## H-103, the open freeze
The app has once locked up so that the window stopped responding, Cancel did nothing and SIGTERM was
ignored; the UI's JS thread was spinning at ~91 %. The suspected cause is `rfd`'s gtk3 backend
starting a second permanent GTK main loop the first time a native dialog opens. It has never been
reproduced on demand. **If it happens again, do not kill the app** — capture a JS profile from a dev
build and the process state first; that is the missing evidence.

## Where to read next
- `docs/README.md` — the documentation hub, including a non-technical introduction.
- `docs/architecture/overview.md` — how the pieces fit, with diagrams.
- `PROMPT.md` §2 — the locked decisions that constrain everything.

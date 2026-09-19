# H-98 — The unreproduced half of the export freeze

- **Tier:** Sonnet. **Open on purpose** — H-96 fixed the cause of the stale panel but could not reproduce the freeze itself.
- **What is settled (H-96):** the terminal `job_progress` event was lost because the listener attached after the job started. Fixed in all four job paths, with a `job_status` fallback, start/finish logging, and a real latent bug found in bake's early-event buffer.
- **What is NOT settled:** the owner's app was *unresponsive* — the dialog's own Cancel did nothing, the window manager's close did nothing, the process **ignored SIGTERM** and needed SIGKILL, while the WebView held ~72 % of a core and the backend main thread ~25 %. H-96 mounted the real dialog components, wedged a job in `running` for a simulated 60 s with the recovery poll deliberately failing, and **could not reproduce** any JS-level loop or unresponsive Cancel. So either the spin lives outside those components (native WebKitGTK, the frame scheduler, the backend), or it needs the real compiled app to appear.

## Scope (in)
1. **Reproduce it** against the real built app, not a component test. `cargo check -p powervoice-app` from a fresh worktree takes ~75 s, so a real build is more tractable than it sounds.
2. When it reproduces, capture evidence that settles the question: `perf top` or a sampling profile of the WebView and the main process at the moment of the freeze, thread states, and whether the main thread is in JS, in GTK/WebKit, or blocked on the backend. A core dump of the wedged process is acceptable evidence too.
3. Fix what the evidence points at. If it turns out to be upstream WebKitGTK behaviour we cannot fix, say so plainly and document the symptom — an honest "known, caused by X, not ours" beats a silent unresolved ticket.
4. **A GUI repro harness** is the reusable deliverable: this repo has no way to drive the real compiled app and read its CPU/signal behaviour, which is exactly why this could not be chased. Build the smallest thing that would have worked here and document it in `docs/contributing.md`.

## Also worth doing while you are in here (H-96's finding, out of its scope)
`export`, `normalize` and `bake` each number their jobs from their own `AtomicU32::new(1)`, so ids collide across kinds, and `export.svelte.ts::applyJobProgress` never checks `payload.kind`. Never observed, because the UI serialises jobs — but it is a real hole. Either scope ids per kind or check the kind on receipt.

## Tests
Whatever the repro turns out to need, plus a regression test for the job-id/kind hole.

`just check` must pass.

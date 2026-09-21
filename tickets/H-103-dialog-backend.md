# H-103 — Two GTK main loops in one process

- **Tier:** Sonnet. **Needs real interactive verification** — do not land this on reasoning alone.
- **From:** H-98's structural finding while chasing the owner's export freeze.

## What was found
`tauri-plugin-dialog`'s default Linux feature set enables `rfd`'s **gtk3** backend. The first time
any native dialog opens, `rfd`'s `GtkGlobalThread` spawns a **second OS thread**, calls
`gtk_init_check()` again — GTK was already initialised on the main thread by tao/webkit2gtk — and
runs its own `gtk_main_iteration()` loop for the rest of the process's life. That is a second,
permanent, unsynchronised GTK main loop sharing the one process-wide default `GMainContext` and GDK
display connection with the window's own loop. **GTK's threading model does not support this.**

## Why it matters here
The owner's freeze happened **after a native save dialog** (the export destination picker) — i.e.
after exactly the event that spawns that thread. The app then stopped handling input and signals
while burning CPU. That is circumstantial, not proof: H-98 built a standalone probe pinned to the
same `tao`/`rfd` versions and found that **merely opening a dialog and leaving it idle does not
reproduce** the CPU burn or the blocked SIGTERM. So the risk is real and the coincidence is
suggestive, and neither is evidence of cause.

- **Read first:** CLAUDE.md, MEMORY.md (H-96, H-98), `docs/contributing.md`'s new "Reproducing a real-app freeze (GUI harness)" section and `scripts/repro/gui_harness.py`, the `rfd` dependency tree (`cargo tree -p rfd -e features -i`).

## Scope (in)
1. **Reproduce first.** Use the harness. The promising shape is a dialog opened *while a render job
   is running*, or a dialog opened and dismissed and then a job — not a dialog sitting idle, which
   is already ruled out. If it reproduces, capture the thread states and where each loop is parked.
2. **If confirmed:** switch the Linux dialog backend to `xdg-portal` and verify Save, Save As, Open,
   Export and the module-install picker still behave — filters, suggested filename, extension
   handling, cancel. H-98 deliberately did not make this change because it is a real behaviour
   change that needs real verification, and that judgement stands.
3. **If it does not reproduce**, say so and document the structural risk in `docs/architecture/` so
   the next person chasing a freeze finds it in ten minutes instead of a day.

## Note on environment
H-98 could not drive the GUI from an agent sandbox: `hyprctl dispatch` is remapped there to a
restricted Lua API with no focus dispatcher, and the agent's own shell reclaims keyboard focus.
`grim` also hung for it, while it works from the orchestrator's session — so **capture and window
control may need to run from the main session**. Plan for that rather than rediscovering it.

`just check` must pass.

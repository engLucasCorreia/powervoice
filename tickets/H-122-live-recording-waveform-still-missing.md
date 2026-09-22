# H-122 — While recording, the waveform still doesn't appear in the real app

- **Tier:** Opus
- **Owner-tested the v0.4.1 test build (main c04d66b) on 2026-09-22: H-114 did not fix it.** Owner: "the waveform does not show up while recording, similar to before. check again." H-114 changed the live view's minimum window (10 s → 2 s), doubled the poll to 20 Hz and extended the wave to the record head each frame — all verified only in the preview harness / Chromium, never in the real app.
- The real app runs in **WebKitGTK** with the **WebGL2** renderer by default (`renderer_preference: Auto`); the preview harness defaults to Canvas2D and fakes the backend. Either the live data never reaches the renderer in the real app (backend peaks during a take, IPC, the store), or the WebGL2 renderer doesn't redraw/re-upload as the take grows (a cached texture/buffer keyed on something that doesn't change during recording).

## Scope (in)
1. **Trace the real path end to end**, from the engine's recording writer → live peaks (whatever produces them during a take) → the Tauri command/event → the UI store → the WebGL2 and Canvas2D renderers. Write down, per hop, what proves data flows during a take. Find the hop that breaks.
2. Fix it so the waveform grows from the first word, smoothly, on WebGL2 and Canvas2D.
3. Tests at the broken hop: a Rust test that live peaks are readable while a take is being written (if that's the hop), and/or a renderer test that a growing take changes what the WebGL2 path draws (not just what the store holds).

**Verification in WebKitGTK:** PyGObject with `gi.require_version('WebKit2','4.1')` works here (webkit2gtk-4.1 2.52.6) — load the preview harness with `&renderer=webgl2` in a WebKit2 WebView and snapshot a simulated recording as it grows. Chromium-only evidence is not accepted. Don't launch the real app (the owner's instance is running) and never use the owner's audio devices.

## Evidence required
The broken hop and why H-114's tests passed anyway; WebKitGTK snapshots at two moments of a simulated take.

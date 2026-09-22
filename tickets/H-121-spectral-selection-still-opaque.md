# H-121 — The spectral selection is still an opaque block in the real app, and the readout card follows the drag

- **Tier:** Opus
- **Owner-tested the v0.4.1 test build (main c04d66b) on 2026-09-22: H-113 did not fix it.** Screenshot of the owner's running app: `/tmp/claude-1000/-home-lucas-Documents-dev-audition/e6923653-879f-4025-bf4c-c0fb63498c26/scratchpad/owner-now.png` (3756×2121; spectral pane is the lower half). A selection 0:09.7–0:16.3 shows in the spectral pane as a **flat opaque blue-gray block** — nothing of the spectrogram visible through it. In the waveform pane above, the same selection is a flat gray block too (the waveform drawn white on it).
- Owner's words: "When i select a area in the spectral view, the spectral card comes with the mouse, and the selection is still grayed out that i cant see the spectral graph over the selection. it is still flat gray". The "card" is the time/Hz/dB hover readout box (visible in the screenshot at the cursor) — it keeps following the pointer during a selection drag.

## Why H-113's verification missed it (read this first)
H-113 was verified in **headless Chromium** (WebGL2 via SwiftShader) against the preview harness. The real app runs in **WebKitGTK 2.52** (Tauri on Linux), on a real GPU. WebKit's WebGL differs from Chromium in ways that matter for exactly this: canvas `premultipliedAlpha`/`alpha` context defaults and compositing, blending with a non-premultiplied colour, `preserveDrawingBuffer`. It may also be a different code path entirely (the spectral pane's own renderer or an overlay canvas/DOM element that H-113 never touched). Find out which layer actually paints that block in the real app before changing anything.

**WebKitGTK is available here for verification:** PyGObject with `gi.require_version('WebKit2','4.1')` works (webkit2gtk-4.1 2.52.6). Load the preview harness (`&renderer=webgl2`) in a WebKit2 WebView from a small Python script in your scratch area and take a snapshot (`WebView.get_snapshot`) — that is the same engine as the app. Chromium-only evidence is not accepted for this ticket. Don't launch the real app (the owner's instance is running) and don't touch it.

## Scope (in)
1. In the spectral pane, a selection is a translucent tint with a clear edge; the spectrogram stays readable under it, in WebKitGTK with WebGL2 **and** with Canvas2D. Same for the waveform pane if it shows the same flat block (the screenshot suggests it does).
2. The hover readout card hides while a selection drag is in progress (and reappears on hover after release), in both panes if both have one.
3. A regression test that would have caught this: at minimum a pixel test of the tinted region (the underlying content must change the output pixel), run through whatever path matches what WebKit does (e.g. premultiplied-alpha maths in a unit test of the colour you actually upload), plus the drag-hides-readout behaviour.

## Evidence required in your report
Before/after WebKitGTK snapshots of the spectral pane with a selection (paths), and the cause in one paragraph.

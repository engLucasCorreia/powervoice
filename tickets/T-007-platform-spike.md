# T-007 — Platform spike: renderers, Wayland input, IPC throughput → ADR-009

- **Milestone / wave:** M0 / W3
- **Tier:** Sonnet (+ Opus review)
- **Depends on:** T-004
- **PROMPT refs:** §3.2, §4 (performance targets) · **ADR refs:** ADR-003

## Goal
Measured evidence, on the owner's machine (Arch, Hyprland/Wayland, AMD Phoenix/Mesa, WebKitGTK 4.1), for choosing the waveform/spectrogram rendering approach — before M2 builds on it.

## Scope
**In:**
- A dev-only spike view (route or `?spike` flag, excluded from production UI) with:
  1. **Waveform**: synthetic min/max peak pyramid for a 60-min 48 kHz file (generated in Rust, delivered via `ipc::Channel` as binary). Two renderers: WebGL2 and Canvas2D. Automated zoom sweep (whole file → ~1 sample/px → back, 10 s) measuring frame times (p50/p95/p99, dropped frames).
  2. **Spectrogram**: 1024×512 u8 magnitude texture scrolled/updated per frame with a colormap in WebGL2 (and a Canvas2D `putImageData` fallback), frame times.
  3. **IPC throughput**: 10 MB via `Channel` and via `Response`, measured end-to-end in ms.
  4. **Input log**: on-screen log of keydown/keyup (Space, Shift+Space, Ctrl+Z, Ctrl+Shift+Z) and pointer drag coordinates vs element-relative position — for the owner to verify manually on Hyprland.
- The spike auto-runs on load when launched with `POWERVOICE_SPIKE=1` and writes results as JSON through a Tauri command to `bench-results/spike-<timestamp>.json`, then keeps the window open for manual input tests.
- Run it with and without `WEBKIT_DISABLE_DMABUF_RENDERER=1` and record both.
- Write `docs/adr/ADR-009-renderer-choice.md`: measurements table, decision (renderer per view), fallbacks, env workarounds, how production code will detect/choose.

**Out:** production renderers (M2), selection logic.

## Acceptance
- [ ] JSON results for both env configurations committed as an appendix in ADR-009 (numbers, not files).
- [ ] ADR-009 decision meets PROMPT §4 target (60 fps scroll/zoom) or documents the gap + mitigation.
- [ ] Manual input checklist included in the ADR for the owner to complete at the M0 checkpoint.
- [ ] `just check` green.

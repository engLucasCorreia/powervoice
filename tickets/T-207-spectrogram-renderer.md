# T-207 — Spectral pane: WebGL2 tile renderer, colormaps, log/linear axis, frequency ruler, hover, Shift+D split

- **Milestone / wave:** M2 / W3
- **Tier:** Sonnet
- **Depends on:** T-204, T-205
- **Spec refs:** SPEC-007 (display ACs: split pane hidden by default, Shift+D, 50/50 draggable divider, log axis 20 Hz–Nyquist + frequency zoom, floor −120 / ceiling 0 dB, Inferno/Viridis/Grayscale, hover readout, zoom sync with the waveform, disabled during import, frozen during recording, GPU memory cap 192 MiB, frame-time targets) · **ADR refs:** ADR-003 `VXST` + Amendment 1 (PREVIEW), ADR-009 · MEMORY A-003

## Goal
The spectral view shows the document as SPEC-007 specifies, in lockstep with the waveform, and never
refetches for display-only changes.

## Scope
**In:** R8 texture upload per tile, shader colormap + floor/ceiling + log/linear frequency mapping, PREVIEW → full tile replacement, tile placement from `first_frame_center_sample`/hop, frequency ruler, hover readout (time, Hz, dB), split pane with draggable divider and Shift+D (keymap registry), GPU memory LRU, Canvas2D fallback, stale/out-of-order rejection.

**Out:** tile computation (T-204), analyzer (T-208).

## Acceptance tests to write
- [ ] SPEC-007 display ACs per its test plan (pure TS mapping functions, Vitest with mocked tiles, automated frame-time run reported).

## Definition of Done
- [ ] `just check` green; report in CLAUDE.md format.

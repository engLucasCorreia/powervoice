# H-13 — WebGL2 renderers (ADR-009 primary path)

- **Tier:** Sonnet + Opus review (renderer correctness/perf; blocking findings only)
- **Depends on:** T-207, H-12 (shared ruler/scrollbar), S1-03/H-07 (waveform).
- **Read first:** CLAUDE.md, MEMORY.md (T-204/T-207 notes: `tile.data` is frame-major — upload width = bins, height = frames and swap axes in the shader; pure math in `colormap.ts`/`sampler.ts`), docs/adr/ADR-009-renderer-choice.md (+ Amendment 1: WebKit DMA-BUF default), specs/SPEC-006 and SPEC-007 renderer sections, ui/src/lib/spectrogram/*, ui/src/lib/waveform/*.

## Goal
Smooth zoom/scroll on long files and HiDPI screens: GPU rendering with WebGL2, Canvas2D kept as the automatic fallback (context creation failure, lost context, or a setting).

## Scope (in)
- Spectrogram: tiles as R8 textures, colormap as a 256×1 LUT texture, floor/ceiling/scale in uniforms; max-or-interpolate sampling per SPEC-007 §2.3/§2.4 (matching `sampler.ts` output); FFT-size texture-limit gating live (`isFftSizeDisabled`).
- Waveform: min/max columns + raw polyline via WebGL2 (instanced quads or line strips).
- Context loss → recreate or fall back to Canvas2D without a blank view.

## Tests
Pure-math parity tests (shader math mirrored in TS vs `sampler.ts`/`colormap.ts`), fallback selection logic, context-loss handling (mocked); a manual owner smoke on real hardware is noted in the report.

## Out
Analyzer overlay (T-208).

`just check` must pass.

# H-12 — Spectral view follow-ups (T-207)

- **Tier:** Sonnet (no review loop; blocking findings only)
- **Depends on:** T-207 (spectral view), T-204 (tile service), H-11 (App/record UI), T-202 if merged (import progress state).
- **Read first:** CLAUDE.md, MEMORY.md (T-204/T-207 notes, A-014, S1-03 `bind:this` gotcha), specs/SPEC-007 §2.1/§2.3/§2.10 (layout, settings), src-tauri/src/settings.rs, ui/src/lib/layout/EditorView.svelte, ui/src/lib/waveform/WaveformView.svelte, ui/src/lib/spectrogram/SpectralView.svelte, ui/src/lib/state/{spectral,settings}.svelte.ts.

## Scope (in)
- **Persist display prefs (A-014):** colormap, frequency scale, floor/ceiling, FFT size → new `Settings` fields (Rust `settings.rs` + `just gen-types`), loaded at start, saved on change (debounced). Per-document view state (pane visibility, split ratio) stays in memory until the T-306 sidecar.
- **Shared ruler + scrollbar:** lift the time ruler and scrollbar out of `WaveformView` into `EditorView` so the stack is ruler → waveform → divider → spectral → scrollbar (SPEC-007 §2.1); both panes keep sharing one viewport.
- **HiDPI:** draw spectrogram columns at device pixels (the tile `hop` is already device-pixel based).
- **Import freeze message:** if T-202's import progress state exists, show SPEC-007's "importing…" frozen overlay in the spectral pane.

## Tests
Settings round-trip (Rust + Vitest), layout order + one scrollbar driving both panes, device-pixel column count at devicePixelRatio 2.

## Out
WebGL2 (H-13), analyzer (T-208).

`just check` must pass.

# S1-03 — Open/Save WAV in the app + Canvas2D waveform view (Slice 1)

- **Tier:** Sonnet
- **Depends on:** S1-01 (engine + transport + telemetry), S1-02 (WAV + peaks libraries)
- **Specs (essential subset only):** SPEC-006 rendering from `VXPK` (≤ 4 buckets/px reduction, raw samples below 64 spp), horizontal zoom (Ctrl+wheel cursor-centred, `=`/`-`), scroll, amplitude ruler optional, time ruler (timecode), playhead drawn from the S1-01 extrapolation helper, click to set the cursor / seek; SPEC-005 File Open/Save/Save As (WAV only); SPEC-004 §2.6 modified mark `*` and unsaved-changes prompt (simple version). ADR-009 renderer: start with **Canvas2D** (WebGL2 in hardening).

## Goal
The owner opens a WAV, sees its waveform, zooms/scrolls, clicks to place the playhead, plays it, and
saves it (Save / Save As WAV 16/24/32f).

## Scope (in)
- `src-tauri`: `tauri-plugin-dialog` for Open/Save dialogs (ADR-007 Amendment 2); commands `document_open(path)` (engine job: import via S1-02 → `Engine::set_document`), `document_save`, `document_save_as(path, bits)`, `peaks_get` (→ `ipc::Response` `VXPK`), `document_changed` event (name, rate, len, dirty, audio_rev).
- UI: File menu / toolbar (Open, Save, Save As with bit-depth choice), title "name — PowerVoice" with `*` when modified, unsaved-changes prompt on open/quit (Save / Don't Save / Cancel); waveform panel (Canvas2D, HiDPI aware, dark theme tokens) with zoom, scroll bar, time ruler, playhead + cursor, click-to-seek, stale response rejection by `audio_rev`; keymap Ctrl+O, Ctrl+S, Ctrl+Shift+S (provisional).
- Tests: Vitest for zoom math (pixel ↔ sample exact), peaks request selection, open/save flows with mockIPC; Rust integration: open fixture WAV → peaks → save → bytes identical for 24-bit source.

## Out (deferred to hardening)
Spectral view, selection editing (Slice 2), WebGL2 renderer, overview strip, vertical zoom, markers display, other formats, recent files, progress UI for long imports.

## Definition of Done
`just check` green; `just dev` smoke: open `fixtures/generated/long-60min-48k-mono.wav` (run `just fixtures` first) and report time to first waveform; report in CLAUDE.md format incl. deferred list.

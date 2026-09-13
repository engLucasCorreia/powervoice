# H-07 — Live waveform while recording (S1-04 gap)

- **Tier:** Sonnet (no review loop; blocking findings only)
- **Depends on:** S1-03, S1-04 (merged), S1-03 fix (waveform viewport observer).
- **Read first:** CLAUDE.md, MEMORY.md (S1-04 learnings, Engine API notes), tickets/S1-04*.md (the "Live waveform while recording" bullet), crates/engine/src/{record.rs,capture.rs}, src-tauri/src/{recording.rs,document.rs}, ui/src/lib/waveform/*, ui/src/lib/state/record.svelte.ts.

## Goal
While recording, the owner sees the take's waveform grow in the editor (owner report: "I can't see the waveform when I am recording").

## Today
The take lives only in the take files and is committed to the document at Stop, so the document is empty during recording and the view shows nothing.

## Scope (in)
- **Engine / capture-writer thread (not the RT callback):** keep a running min/max peak list for the open take at a fixed bucket size (e.g. 256 samples per bucket; allocate on the writer thread, never in the audio callback). Expose a cheap snapshot, e.g. `EngineHandle::live_take_peaks(start_bucket, max) -> Option<LiveTakePeaks { sample_rate_hz, len_samples, spb, buckets }>`. A lock that only the writer thread and the control/IPC side take is acceptable; the RT thread must never touch it.
- **src-tauri:** a thin `record_peaks_get(start_bucket, count)` command returning VXPK framing (reuse `encode_vxpk`, `partial: true`), or reuse `peaks_get` when a recording is active. Only forwarding, no logic.
- **UI:** while `recordState` is recording, poll at ~10 Hz and draw the growing take in `WaveformView`, which is now shown even with no committed document. View fitting: zoom so the take fills the view, growing (at least the last N seconds visible), plus a record-head line. On Stop, the normal `document_changed` path takes over (already works).
- **Tests:**
  - Rust: the peak list matches min/max of the appended samples; no allocation in the audio callback (existing RT tests stay green).
  - Vitest: while recording, the view polls and renders the canvas without a committed document.

## Out
The `VXRP` channel push, overdub/punch-in display, spectral view while recording.

`just check` must pass.

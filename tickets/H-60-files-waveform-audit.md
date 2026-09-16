# H-60 — Save pipeline, peaks and waveform renderer audit (T-201/T-203/T-205 remainder)

- **Tier:** Sonnet (audit first, then fill gaps; blocking findings only)
- **From:** three rows marked "subset, rest hardening": the save pipeline (WAV 16/24/32f, TPDF dither, clip policy, cue/adtl markers, atomic save, FLAC encode), the per-chunk peak pyramid and its binary IPC, and the waveform renderer (zoom/scroll, amplitude zoom, rulers, extrapolated playhead).
- **Method:** audit against the specs first, matrix in your report, then fill the real gaps.
- **Read first:** CLAUDE.md, MEMORY.md (S1-02/S1-03 slices, T-704 peaks/import speedups, H-35 zoom, H-27/H-23 follow, H-43 frame scheduler, H-47 upload budget, T-206 time formats), specs/SPEC-005 (files, formats, dither, clip policy), SPEC-006 (waveform view ACs), SPEC-009 (markers in files), SPEC-018 (sidecar), `crates/io`, `crates/project` (peaks, store), `ui/src/lib/waveform/*`.

## Scope (in)
1. Save pipeline: every bit depth, TPDF dither on and off, the clip policy, cue/adtl marker round trips, atomic save under failure (disk full, permission denied), and FLAC encode options the spec names.
2. Peaks: pyramid correctness at every level, the binary IPC frame and its golden fixture, behaviour during import.
3. Waveform renderer: the SPEC-006 acceptance criteria not already covered by H-35/H-43/H-47 — amplitude zoom limits, ruler formats, the extrapolated playhead under load, and behaviour at extreme zoom.

## Tests
One per gap; golden fixtures where a binary frame lacks one.

`just check` must pass.

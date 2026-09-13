# S4-02 — Export: WAV / FLAC / MP3 with ACX preset (Slice 4)

- **Tier:** Sonnet (one Opus review of the LAME FFI + resampler path, blocking findings only)
- **Depends on:** S1-03 (save plumbing), S1-02 (WAV writer), T-103 (offline render)
- **Requirements (lean; hardening refines via SPEC-005 [M6] items):**
  - Export renders the **rack** over the whole file or the selection (`rack::offline::render`, time-aligned), then converts: sample rate (keep or 44.1/48 kHz via `rubato` sync FFT in `vox-dsp`), bit depth with TPDF dither (reuse S1-02), format WAV 16/24/32f, FLAC (`flacenc`, block 4096), MP3 via **LAME loaded at runtime with `libloading`** (ADR-007: our own minimal FFI table; system `libmp3lame.so` on Linux — Arch package `lame`; if missing, MP3 is disabled with a clear message), CBR 128–320 kbps, VBR V0–V4.
  - **ACX preset:** MP3 CBR 192 kbps, 44.1 kHz, mono.
  - Export never modifies the document; runs as a cancellable job with progress; writes atomically.
  - References: PROMPT §3.5, ADR-007 (+ Amendments 1–2), MEMORY A-003 (resampling quality targets: ±0.1 dB passband to 0.9×Nyquist, aliasing ≤ −100 dBFS — measure and report, don't block on it).
- **Read first:** CLAUDE.md, MEMORY.md (D-013 LAME approved, D-022), ADR-007.

## Goal
The owner exports the processed voice-over as a delivery file — including an ACX-compliant MP3 — in one dialog.

## Scope (in)
`vox-io` FLAC encoder + LAME dynamic loader (feature-gated FFI, no LAME code compiled in); engine export job; `export_formats` (reports MP3 availability), `export_start/cancel` commands + progress; Export dialog (format, rate, bit depth/bitrate, range whole/selection, ACX preset button, destination via tauri-plugin-dialog).

## Tests
Export a 1 kHz −20 dBFS sine through `[Gain −6 dB]`: WAV 24 → peak −26.00 ± 0.01 dBFS; FLAC decodes bit-exact to the 24-bit WAV; 48 k → 44.1 k resample keeps a 1 kHz tone at 1 kHz ± 0.05 % and level ± 0.1 dB; MP3 (if libmp3lame present) decodes with symphonia to the right rate/duration ± 1 frame and level ± 0.5 dB (ffprobe check in the report: 44100 Hz, 192 kb/s CBR, mono).

## Out (deferred)
Markers/metadata in exported MP3/FLAC, batch export, Opus/AAC export, noise-shaped dither.

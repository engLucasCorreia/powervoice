# S4-04 — Export job + dialog (Slice 4)

- **Tier:** Sonnet
- **Depends on:** S1-03 (app document/save plumbing), S4-02 (encoders), T-103 (offline render)
- **Requirements:** as in S4-02's original brief — export renders the **rack** over the whole file or the selection (`rack::offline::render`, time-aligned), converts rate/bit depth/format with the S4-02 encoders, never modifies the document, runs as a cancellable job with progress, writes atomically. **ACX preset:** MP3 CBR 192 kbps, 44.1 kHz, mono. If "Output noise only" is on in an NR slot, confirm before exporting (SPEC-014).

## Scope (in)
Engine export job; commands `export_formats` (reports MP3 availability), `export_start`, `export_cancel` + progress events; Export dialog (format, rate, bit depth/bitrate, range whole/selection, ACX preset button, destination via tauri-plugin-dialog), notices.

## Tests
Export a 1 kHz −20 dBFS sine through `[Gain −6 dB]`: WAV 24 → peak −26.00 ± 0.01 dBFS; FLAC decodes bit-exact to the 24-bit WAV; 48 k → 44.1 k keeps 1 kHz ± 0.05 % and level ± 0.1 dB; MP3 (if libmp3lame present) decodes to the right rate/duration ± 1 frame and level ± 0.5 dB (report `ffprobe`: 44100 Hz, 192 kb/s CBR, mono).

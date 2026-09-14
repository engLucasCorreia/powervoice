# H-20 — Save/open gaps (SPEC-005, after T-209)

- **Tier:** Sonnet (no review loop; blocking findings only)
- **Depends on:** T-209 (import job, channel choice, Save As format, clip prompt), H-02 (dither in vox-dsp), H-14 (FLAC verify).
- **Read first:** CLAUDE.md, MEMORY.md (T-202, T-209, H-02, H-14 notes; `saved_file_crc32` must hash what a later open decodes; DTO literal gotcha / H-18 fixtures if merged), specs/SPEC-005 (save format table, dither modes, notices, multichannel save warning, progressive import), crates/dsp/src/dither.rs, crates/project/src/{save.rs,import.rs}, src-tauri/src/document.rs, ui/src/lib/document/*.

## Scope (in)
1. **Dither "None"** mode (`vox_dsp::dither`: plain rounding path; SPEC-005 default stays TPDF) selectable in Save As.
2. **Keep the source format:** an opened FLAC saves as FLAC (same bit depth) by default; the save-format promotion table for WAV variants (8-bit → 16, 32i/64f → 32f) applies to every WAV variant, not only hound-readable ones.
3. **Notices:** `notice.open.format_mapped` (source format/depth mapped to a different save format), `notice.save.metadata_dropped` (tags the source had that we don't write), and the "saving replaces the stereo source with mono" warning before the first save over a multichannel source.
4. **Progressive import display:** the document shell appears immediately with the waveform filling in as chunks import (peaks for imported chunks only), Cancel still leaves the previous document untouched until commit.

## Tests
One per item; FLAC-in → FLAC-out bit depth kept; progressive import shows partial peaks then the full document; Cancel mid-import restores the previous document.

## Out
Metadata tag writing (ID3/Vorbis comments) — a later ticket.

`just check` must pass.

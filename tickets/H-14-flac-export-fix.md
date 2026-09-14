# H-14 — FLAC export is malformed (fix or replace the encoder)

- **Tier:** Sonnet (no review loop; blocking findings only)
- **Depends on:** S4-02 (FLAC encoder via `flacenc`), T-202 (symphonia decoder), H-02 (dither/quantizer in vox-dsp).
- **Read first:** CLAUDE.md, MEMORY.md (S4-02 note: flacenc 0.5.1 output makes `flac -t` warn "frame number does not increase correctly… might not be seekable"; T-202 note: symphonia rejects our own FLAC exports with `UnexpectedEof` on its sample-count cross-check, while the reference `flac` CLI's output decodes fine through the same path; H-02 streaming/quantizer notes), docs/adr/ADR-007-licensing.md (encoder options), specs/SPEC-005 FLAC sections (§2.11 verify-before-rename), crates/io/src/{flac.rs,encoder.rs,decode.rs}.

## Goal
FLAC files exported by PowerVoice are valid, seekable, and re-open in PowerVoice and in other players.

## Scope (in)
1. **Find the root cause** in how we drive `flacenc` (block size / variable vs fixed blocking strategy / frame or sample numbering in frame headers / STREAMINFO min/max block size / total samples / MD5). Reproduce with a minimal test first.
2. **Fix it** — in order of preference: correct our configuration/usage of `flacenc`; or post-process the frame headers (rewrite frame numbers + CRC-8/CRC-16) if the crate itself is at fault; or replace the encoder with an ADR-007-compatible option (report and justify before adding a dependency).
3. **Verify-before-rename (SPEC-005 §2.11):** after writing a FLAC export, decode it with the T-202 decoder and compare sample count (and a hash of the quantized samples) before the atomic rename; on mismatch, fail the export with a clear error and keep the temp file out of the way.

## Tests
- `flac -t` passes with no warnings on our output (skip gracefully if `flac` is missing) — multi-frame files, 16- and 24-bit, 44.1/48/96 kHz.
- symphonia decodes our export bit-exactly to the quantized input (same dither seed).
- A deliberately corrupted temp file makes verify-before-rename fail the export.

## Out
FLAC metadata tags (Vorbis comments), seek-table tuning.

`just check` must pass.

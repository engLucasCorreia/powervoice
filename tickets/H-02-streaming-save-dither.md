# H-02 — Streaming save + dither moves to vox-dsp

- **Tier:** Sonnet (no review loop; blocking findings only)
- **Depends on:** S1-02/S1-03 (WAV save), S2-03 (markers written as cue/adtl), S4-02 (encoders share the TPDF quantizer).
- **Read first:** CLAUDE.md, MEMORY.md (S1-02, S2-03, S4-02 notes; `/tmp` quota gotcha), ADR-001 §4 (dither lives in `vox-dsp`), SPEC-005 save sections, crates/project/src/save.rs, crates/io/src/{wav.rs,flac.rs,encoder.rs}, crates/project reader (SnapshotReader).

## Goal
Saving a long recording doesn't hold the whole document in RAM (today ~691 MB for 60 min mono f32), and dither has one home.

## Scope (in)
- `save_snapshot_wav` streams the snapshot in fixed-size blocks (e.g. 64k samples) through the WAV writer; memory stays bounded regardless of length. Markers still written as cue/adtl after the data (S2-03 path).
- Move the TPDF dither/quantizer from `vox-io` into `vox-dsp::dither` (ADR-001 §4); `vox-io` WAV/FLAC writers call it. Behavior unchanged (same seedable RNG → identical output for the same seed).

## Tests
- Saved file is bit-identical to the pre-change output for f32 and for 16/24-bit with the same dither seed (golden/hash).
- Streaming: a document larger than several blocks saves correctly (FNV-1a of samples round-trips); the save path never allocates a buffer proportional to document length (assert block reads via a counting reader, or a size bound in a test hook).
- Markers survive the streamed save.

## Out
Save-as-job progress UI (T-201 hardening), verify-before-rename for FLAC.

`just check` must pass.

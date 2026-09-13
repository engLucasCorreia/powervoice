# T-201 — Save pipeline: WAV writer, TPDF dither, clip policy, cue/adtl markers, atomic save job, FLAC encode

- **Milestone / wave:** M2 / W1
- **Tier:** Sonnet (+ Opus review)
- **Depends on:** M1 (T-101 document model, T-105 engine jobs)
- **Spec refs:** SPEC-005 (save/Save As, save format, dither, clipping, markers ↔ `cue`/`LIST adtl`, FLAC, atomic save; the ACs tagged for saving/markers/FLAC), SPEC-004 AC-1 (save of revision r is isolated from concurrent edits), AC-14 (atomic save) · **ADR refs:** ADR-004 (save = render snapshot → temp + rename), ADR-001 (codecs only in `io`), ADR-007 (+ Amendment 2)

## Goal
A document revision can be saved to WAV (16/24/32f) or FLAC exactly as SPEC-005 specifies — bit-exact
when nothing needs to change, TPDF-dithered when reducing bit depth, clip-safe, with markers written to
`cue`/`LIST adtl` — atomically and without blocking editing.

## Scope
**In:**
- `vox-io`: WAV writer on `hound` (PCM tag for 16/24, float for 32f) + custom RIFF writer/reader for `cue ` and `LIST adtl` (`labl`, `ltxt` `'rgn '` for regions; UTF-8 names, CP-1252 read fallback), placed after `data`; no RF64 writing (> 4 GiB → error with notice per SPEC-005).
- TPDF dither (seeded, deterministic per revision) with the exactly-representable-block skip; no noise shaping. Implemented in `vox-dsp` (`dither`), used by `io`.
- Clip policy: detect samples > 1.0 for integer targets and return a structured result the UI turns into the prompt (UI is T-209).
- FLAC encode via `flacenc` (block size 4096) + decode-compare verification before replacing the target.
- Engine save job: render snapshot r via `vox-project` reader → temp file next to target → fsync → atomic rename; stale temp cleanup rule (SPEC-004 §2.6); progress + cancel; `document_save` / `document_save_as` IPC commands (DTOs, ts-rs) returning structured outcomes (clip needed, metadata dropped notice, success).
- `powervoice-cli markers <file>` (list cue/adtl markers) for tests.

**Out:** import/open (T-202), dialogs/UI (T-209), MP3 export and export resampling (M6), sidecar (T-306).

## Acceptance tests to write
- [ ] Every SPEC-005 AC for save/format/dither/clip/markers/FLAC/atomicity, per its test plan (e.g. 16-bit TPDF residual −96.33 ± 0.3 dBFS; open→save bit-exact for unchanged 16/24-bit sources; digital silence stays silent; markers round-trip exactly in samples and names).
- [ ] SPEC-004 AC-14 (kill at 20 random points during save: target old or complete, never partial) and AC-1 save part.

## Definition of Done
- [ ] Tests first, passing; `just check` green; Opus review passed; report in CLAUDE.md format.

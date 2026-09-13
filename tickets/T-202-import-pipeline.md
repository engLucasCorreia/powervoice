# T-202 — Import pipeline: symphonia decode, downmix, session import job, CLI convert/markers

- **Milestone / wave:** M2 / W1
- **Tier:** Sonnet (+ Opus review)
- **Depends on:** M1 (T-101, T-105)
- **Spec refs:** SPEC-005 (open/import, supported and rejected formats, WAV variants with notices, downmix policy and dialog data, errors, import progress, encoder delay/padding trimming, the ACs for opening), SPEC-004 OD-3 (import cost; < 3 s for 60 min WAV on SSD with progressive peaks) · **ADR refs:** ADR-004 (open = import into the session store), ADR-007 Amendment 2 (symphonia features, committed codec test vectors), ADR-001

## Goal
Any supported file opens into a document as SPEC-005 says: decoded, downmixed to mono by the chosen
policy, imported into the session store with progressive peaks, with honest notices and errors.

## Scope
**In:**
- `vox-io` decoder on `symphonia` 0.6 (`default-features = false`; `wav`, `pcm`, `flac`, `ogg`, `vorbis`, `mp3`, `isomp4`, `aac`) — WAV variants incl. 8-bit, 32-bit int, 64-bit float, A-law/µ-law (verify coverage; implement small custom readers for any variant symphonia lacks), FLAC, MP3, M4A (AAC-LC), Ogg Vorbis; rejection list with specific errors; encoder delay/padding trim via `Track.delay/padding`; read `cue`/`LIST adtl` markers (reuse T-201's RIFF reader if merged first, else implement the reader here and T-201 reuses it — coordinate through the orchestrator).
- Downmix: average (LFE excluded) or pick channel; the detection data SPEC-005 needs for the dialog (silent channel in first 30 s, bit-identical channels). Probe command returns channel info so the UI (T-209) can ask before importing.
- Engine import job: streams decoded audio through `ChunkWriter` into a new session, emitting `peaks_progress`; cancel; `document_probe` and `document_open` IPC commands (DTOs, ts-rs).
- Committed codec test vectors (≤ 512 KiB) under `crates/io/tests/data/` + `just fixtures-codec` recipe that regenerates them (document the generator; if no encoder is available on this machine, generate what is possible and report the rest).
- `powervoice-cli convert <in> <out.wav> [--downmix avg|ch:N]` using the same import path.

**Out:** save (T-201), UI dialogs (T-209), peaks IPC (T-203).

## Acceptance tests to write
- [ ] Every SPEC-005 AC for opening/import/downmix/errors, per its test plan (e.g. L=+sine/R=−sine averages to digital silence; rate preserved; notices for WAV variants; rejected formats return the specific error key).
- [ ] 60-min 48 kHz 24-bit WAV import (ignored-by-default perf test): first peaks available < 1 s, import complete < 3 s on the owner's NVMe (report numbers).

## Definition of Done
- [ ] Tests first, passing; `just check` green; Opus review passed; report in CLAUDE.md format incl. the T-200 verification items (symphonia PCM coverage, RF64 read, HE-AAC, flacenc MD5).

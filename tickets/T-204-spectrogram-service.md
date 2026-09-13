# T-204 — Spectrogram tile service: STFT tiles, zoom-dependent hop, power averaging, PREVIEW tiles, cache

- **Milestone / wave:** M2 / W1
- **Tier:** Opus (+ Opus review)
- **Depends on:** M1 (T-101, T-105 worker pool, T-108 channel patterns)
- **Spec refs:** SPEC-007 (spectrogram ACs: FFT sizes/Auto, Hann window, frame spacing vs zoom, power averaging, PREVIEW then full tiles, quantization, cache/memory cap, latency targets, cancellation) · **ADR refs:** ADR-003 §2 `VXST` + Amendment 1, ADR-004 (snapshots, content keys), ADR-001 (`realfft` only in `dsp`)

## Goal
Spectrogram tiles for any visible region arrive quickly and correctly at any zoom, are cached by content,
and never block playback or editing.

## Scope
**In:**
- `vox-dsp`: STFT frame computation (realfft, Hann, magnitude → dB normalized so a full-scale sine's peak bin reads 0 dB), power averaging over a frame span for zoomed-out views, quantization to u8 per ADR-003.
- Engine tile service on the worker pool: `spectro_attach(view_id, channel)`, `spectro_request(view_id, SpectroRequest)`; visible tiles first; PREVIEW tile then full tile; newer `request_id` cancels unsent tiles; content-keyed LRU cache capped at 25 % of the memory budget; `audio_rev` stamping; golden `VXST` fixture + Vitest decode test.
- Performance: visible tiles < 200 ms after a zoom on a 60-min document (SPEC-007), measured in an ignored bench.

**Out:** rendering (T-207), live analyzer (T-208).

## Acceptance tests to write
- [ ] SPEC-007 spectrogram ACs per its test plan (bin-centred tone level ± 0.35 dB, peak within one bin, tile hash stability, cancellation, cache reuse after marker-only edits, re-key after length-changing edits).
- [ ] No allocation or blocking on the audio thread while tiles compute (fake-backend playback + heavy tile load: 0 underruns).

## Definition of Done
- [ ] Tests first, passing; `just check` green; Opus review passed; report in CLAUDE.md format.

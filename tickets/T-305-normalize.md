# T-305 — Peak normalize favorites and Normalize… dialog

- **Milestone / wave:** M3 / W2
- **Tier:** Sonnet (+ Opus review)
- **Depends on:** T-301
- **Spec refs:** SPEC-010 (all ACs), SPEC-004 (AC-2 all-or-nothing, AC-6 stop-before-commit) · **ADR refs:** ADR-004 (ChunkWriter, progress/cancel) · MEMORY A-002 and the T-305 follow-ups

## Goal
One-click −1 / −0.1 / −3 dB peak normalize and a custom Normalize… dialog, exact to ±0.01 dB,
cancellable, undoable, never clipping.

## Scope
**In:** peak scan over selection or whole file (brute force; an optional pyramid-accelerated scan only if proven bit-exact vs brute force per SPEC-010 AC-15, with a "no non-finite samples" guarantee at chunk commit), gain computed in f64 applied per sample then stored as f32 through `ChunkWriter`, no-op rules (−inf / < −120 dBFS / |gain| < 0.001 dB) and non-finite refusal with notices, playback stop at command start, progress + cancel, undo label with params ("Normalize to −1 dB"); UI: three toolbar favorite buttons, Favorites menu, Effects → Normalize… dialog (dB or %, remembered in settings), one-time-per-session rack notice.

**Out:** LUFS / true-peak normalize (M6), DC offset.

## Acceptance tests to write
- [ ] SPEC-010 AC-1…AC-15 per its test plan (testkit `peak_dbfs` −1.00 ± 0.01 etc.; null test of scaled original ≤ −120 dB; outside-selection bit-identical; cancel leaves the document unchanged; 60-min ≤ 10 s bench, ignored by default).

## Definition of Done
- [ ] Tests first, passing; `just check` green; Opus review passed; report in CLAUDE.md format.

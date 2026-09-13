# S4-01 — Loudness analysis + LUFS normalize (Slice 4)

- **Tier:** Sonnet (one Opus review of the measurement code, blocking findings only)
- **Depends on:** S2-02 (normalize plumbing), S1-01
- **Requirements (lean; no separate spec yet — SPEC-011 is written in hardening):**
  - Measure per ITU-R BS.1770-5 / EBU R128 for mono (channel weight G = 1.0, no dual-mono +3 dB by default — PROMPT §3.3): integrated LUFS, max short-term, max momentary, LRA, sample peak, true peak (4× oversampled). Use the `ebur128` crate (feature `precision-true-peak`) inside `vox-dsp` (production), measuring the **processed output** (rack rendered offline via `rack::offline::render`) by default, with a "source" toggle.
  - LUFS normalize favorites −16, −19, −23 LUFS + custom: gain = target − integrated, applied like peak normalize (one undo entry); if the resulting true peak would exceed −1 dBTP, apply the gain anyway and show a notice suggesting the true-peak limiter (don't silently limit).
  - References: docs/references.md (BS.1770 gating 400 ms/75 %/−70/−10; mono 1 kHz −20 dBFS ≈ −23.0 LUFS); MEMORY gotcha: ebur128 TP reads ~+0.1 dB high near fs/4.
- **Read first:** CLAUDE.md, MEMORY.md (D-022), crates/testkit measurements (independent reference), crates/rack offline render.

## Goal
The owner sees the loudness of the voice-over (as it will be exported) and hits −16/−19/−23 LUFS with one click.

## Scope (in)
Engine analysis job (progress), `loudness_analyze` command → report DTO; Loudness panel (bottom dock) showing I / S-max / M-max / LRA / sample peak / TP; LUFS favorites in the Favorites menu/toolbar; LUFS normalize job reusing S2-02's gain path.

## Tests
testkit cross-check: mono 1 kHz −20 dBFS → −23.0 ± 0.1 LUFS; EBU Tech 3341 cases 1–2 → ± 0.1 LU; after −19 LUFS normalize, re-measured integrated = −19.0 ± 0.1; processed vs source toggle differs by the rack gain (Gain −6 dB → 6.0 ± 0.1 LU).

## Out (deferred)
Live loudness meter during playback, ACX check (S4-03), loudness history graph.

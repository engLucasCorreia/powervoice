# S4-03 — ACX check report (Slice 4)

- **Tier:** Sonnet
- **Depends on:** S4-01 (loudness analysis job, processed-output render)
- **Requirements (lean; SPEC-011 is written in hardening):** ACX / Audible rules from docs/references.md —
  RMS between −23 and −18 dB, peak ≤ −3 dBFS, noise floor ≤ −60 dB (quietest 500 ms window, unweighted,
  per MEMORY testkit conventions), mono/stereo consistency, delivery format hint (MP3 CBR ≥ 192 kbps,
  44.1 kHz — S4-02's ACX preset). Analyze the **processed output** (rack rendered offline) by default,
  with a "source" toggle, reusing S4-01's analysis job.
- **Read first:** CLAUDE.md, MEMORY.md (D-022; testkit measurement conventions: dB = 20·log10(rms),
  −inf silence, None for too-short input), docs/references.md ACX section.

## Goal
One click tells the owner whether the chapter passes ACX and what to fix.

## Scope (in)
Engine: reuse the loudness analysis render; compute RMS (whole file), sample peak, noise floor (quietest
500 ms window); pass/fail per rule with measured values and a short hint (e.g. "Peak −1.2 dBFS > −3 dBFS:
add or lower the true-peak limiter ceiling"). Command `acx_check` → report DTO. UI: "ACX Check" button in
the Loudness panel showing a table (rule, measured, limit, ✓/✗) and the hints; i18n strings.

## Tests
Synthetic files that pass and fail each rule exactly at the boundaries (testkit generators): RMS −18.0/−17.9,
peak −3.0/−2.9, noise floor −60.0/−59.9 (± 0.1 dB tolerance on measurements); processed vs source toggle
reflects a Gain −6 dB rack.

## Out (deferred)
Room-tone head/tail length check, per-file 120 min limit, batch checks over multiple chapters.

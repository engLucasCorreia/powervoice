# S3-06 — Capture Noise Print command + NR slot panel (Slice 3)

- **Tier:** Sonnet
- **Depends on:** S3-01 (rack panel), S3-04 (NR module), S2-01 (selection)
- **Spec:** SPEC-014 [S3] capture workflow: select room tone (≥ 0.5 s; warn < 1 s; analyse ≤ 60 s; error on digital silence; warn above −35 dBFS RMS; disabled while recording) → Effects → Noise Reduction / Restoration → Capture Noise Print (Shift+P, provisional) → target = last-focused NR slot, else first NR slot, else insert one at the top; capture source = the audio as the NR slot receives it (render earlier slots offline with 1 s pre-roll, placeholders skipped); status line + Capture button in the NR slot panel; "Output noise only" header badge.

## Goal
The owner selects room tone, presses Shift+P, and the noise reducer is armed with that noise print.

## Scope (in)
Engine capture job (offline render of preceding slots + `NoiseProfile` capture via `RackHost::replace_state`), `nr_capture` command + notices, NR slot panel additions (Capture button, status: "No noise print" / captured length + level), menu + keymap entry.

## Out
Profile graph, export confirmation for Output-noise-only (with S4-02 or hardening).

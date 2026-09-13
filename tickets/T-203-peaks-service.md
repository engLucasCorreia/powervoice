# T-203 — Peaks service: document-level peak buckets over the piece table, `VXPK` responses

- **Milestone / wave:** M2 / W1
- **Tier:** Sonnet (+ Opus review)
- **Depends on:** M1 (T-101 per-chunk pyramids, T-108 IPC patterns)
- **Spec refs:** SPEC-006 (peaks consumption, PARTIAL buckets, request_id/audio_rev rejection, raw samples below 64 spp), SPEC-000 AC-7 (golden `VXPK` fixture) · **ADR refs:** ADR-003 §2 `VXPK`, ADR-004 §5

## Goal
The UI can fetch any window of the document as min/max buckets (or raw samples) in one binary response,
fast enough for 60 fps zoom/scroll on a 60-minute document, and correct across edits.

## Scope
**In:**
- `vox-project`: document-level buckets for (spp level, start, count) as the conservative union of per-chunk pyramid buckets over the piece table (pieces starting mid-bucket handled exactly — never under-report amplitude); coarser levels derived per request; raw sample path for spp < 64; PARTIAL (`NaN`,`NaN`) for chunks still importing.
- `src-tauri`: `peaks_get(PeaksRequest)` → `ipc::Response` with the `VXPK` layout; server caps (65 536 buckets / 1 Mi samples); `audio_rev` echo; golden fixture generation (`gen_ipc_fixtures`) + `binary.ts` decoder + Vitest contract test.
- Performance: a 4 000-px view request on a 60-min document answers in < 5 ms p95 (bench in the report).

**Out:** rendering (T-205).

## Acceptance tests to write
- [ ] Bucket correctness vs brute force on random piece tables (min/max never inside the true range; exact when pieces are bucket-aligned).
- [ ] PARTIAL buckets during a simulated import; stale `audio_rev` echoed correctly.
- [ ] SPEC-000 AC-7 `VXPK` golden-frame contract (Rust ↔ Vitest).
- [ ] Latency bench (ignored by default) with numbers in the report.

## Definition of Done
- [ ] Tests first, passing; `just check` green; Opus review passed; report in CLAUDE.md format.

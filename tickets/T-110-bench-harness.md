# T-110 — Bench harness: divan benches, callback-time histogram, `powervoice-cli bench`

- **Milestone / wave:** M1 / W3
- **Tier:** Sonnet
- **Depends on:** T-105
- **Spec refs:** PROMPT §4 performance targets · **ADR refs:** ADR-002

## Goal
Performance is measured from M1 on, so M7's tuning (T-704) starts from numbers, not guesses.

## Scope
**In:**
- `divan` benches for: Gain `process` per sample, rack with 1/8/16 TestGain slots, snapshot reader throughput, peak-pyramid computation per chunk.
- Fake-backend callback-time histogram (p50/p95/p99/max per callback at 64/256/1024 frames) for playback with a rack.
- `powervoice-cli bench [--json]` running a fixed suite and printing a table; `just bench` runs divan + the CLI suite and writes `bench-results/<date>.json` (gitignored).

**Out:** optimization work (T-704).

## Crates / files
`crates/*/benches/`, `crates/cli`, `justfile`. Allowed new dep: `divan` (dev-dependency).

## Acceptance tests to write
- [ ] `powervoice-cli bench --json` emits valid JSON with all suite entries (integration test with a short suite flag).
- [ ] `just bench` completes on the owner's machine; numbers included in the report.

## Definition of Done
- [ ] `just check` green; report includes the first baseline table.

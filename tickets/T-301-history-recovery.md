# T-301 — History & recovery: undo/redo with labels, dirty tracking, journal replay, crash recovery, budgets, compaction

- **Milestone / wave:** M3 / W1
- **Tier:** Opus (+ Opus review)
- **Depends on:** M2 (T-201 save for the dirty/saved state; T-101 journal + history basics)
- **Spec refs:** SPEC-004 (M3 ACs: AC-2, AC-3, AC-4, AC-8, AC-9, AC-10, AC-11, AC-12 UI part, AC-13; AC-16 attachment plumbing), SPEC-008 §2.3 (undo/redo cursor rule), SPEC-010 (undo labels with params) · **ADR refs:** ADR-004 (§4 budgets/eviction/compaction, §6 journal/fsync, §9 recovery, §10 GC, Amendment 1 attachment) · MEMORY D-015 (OD-1 drop oldest with notice, OD-2 keep until discarded), follow-ups from T-300

## Goal
Undo/redo is exact, instant and unlimited within the disk budget; every edit that reported success
survives a crash; recovery is a clear dialog; sessions never silently lose or leak data.

## Scope
**In:**
- History with labels + label params (ADR-004 amendment: journal `edit` records carry `{label_key, params}`; write the amendment text into ADR-004 as "Amendment 2" in your branch), redo truncation, dirty vs saved sequence, undo/redo cursor/selection rule (SPEC-008 §2.3), `history_undo` / `history_redo` commands and a `history_state` event (labels, can_undo/can_redo, dirty).
- Journal replay and crash recovery in `vox-project` + engine: classify sessions (clean/recoverable/locked), restore snapshots, undo/redo stacks, labels, attachments; damaged-record handling; interrupted-take handling hand-off to T-106's recovery path ("Apply as recorded" / "Open as new document" / "Discard take").
- Recovery dialog (Svelte) at startup + Settings → Recovery & storage (list, sizes, Clear…, Decide later), all strings i18n.
- Disk budget checks (after each edit and every 10 s idle), compaction by generation (crash-safe), drop-oldest-undo with notice (OD-1 = A), never during recording; memory budget live change from settings.
- Finish session GC rules incl. stale temp files (SPEC-004 §2.6).

**Out:** edit ops (T-302), markers panel (T-303), sidecar (T-306).

## Acceptance tests to write
- [ ] SPEC-004 AC-2, AC-3 (incl. 60-min/20 000-piece timing ≤ 50 ms per undo on SSD, ignored-by-default bench), AC-4, AC-8, AC-9 (50 SIGKILL points + journal fault injection at every offset), AC-10, AC-11, AC-12 (UI part), AC-13.
- [ ] Label params round-trip through the journal ("Normalize to −1 dB").

## Definition of Done
- [ ] Tests first, passing; `just check` green; Opus review passed; report in CLAUDE.md format.

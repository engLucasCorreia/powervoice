# H-74 — Dropout notice edge case and a notice test fixture

- **Tier:** Haiku (small, mechanical — but verify both halves against the code before reporting)
- **From:** H-67's deferrals.
- **Read first:** CLAUDE.md, MEMORY.md (H-67's notice actions and Amendment 7 in ADR-003; SPEC-022 §2.12's dropout filtering), specs/SPEC-022 (§2.12), `crates/engine/` capture commit path (`commit_take_op`), the dropout notice construction, `ui/src/lib/test/fixtures.ts`.

## Scope (in)
1. When SPEC-022 §2.12's filtering removes every dropout from the committed window, no marker is placed — so the notice's "Go to first" action goes nowhere. Have `commit_take_op` report whether it actually placed a marker, and omit the action when it didn't. Keep the notice itself (the user should still know dropouts happened).
2. Add a `noticeFixture` helper to `ui/src/lib/test/fixtures.ts` and adopt it at the literal `Notice` construction sites in UI tests, so the next field added to `Notice` doesn't touch ten files.

## Tests
- A commit where every dropout is filtered out: notice present, action absent. And one where a marker is placed: action present.
- The existing notice tests keep passing through the fixture.

`just check` must pass.

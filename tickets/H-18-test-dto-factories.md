# H-18 — Shared test factories for DTOs (merge-friction fix)

- **Tier:** Haiku or Sonnet (mechanical refactor; no review loop)
- **Depends on:** nothing running in the same files (dispatch when no other agent edits `ui/src/**/*.test.ts` heavily).
- **Read first:** CLAUDE.md, MEMORY.md (H-10/T-301 notes: every new field on a shared DTO forced edits to 15–20 hand-written test literals and caused cross-branch merge conflicts), ui/src/lib/ipc/bindings.ts, the existing `*.test.ts` files.

## Goal
Adding a field to a shared DTO touches one test helper, not every test file.

## Scope (in)
- `ui/src/lib/test/fixtures.ts`: typed factories with complete defaults and overrides — `docDto(over?)`, `recordStateDto(over?)`, `settingsFixture(over?)`, `rackSlotDto(over?)`, `historyStateDto(over?)`, `transportStateDto(over?)` (whichever exist in `bindings.ts`), each returning a full object that `satisfies` the generated type.
- Replace every hand-written literal of those DTOs in `ui/src/**/*.test.ts` with the factory (keep each test's meaningful overrides explicit).
- Rust side: the same for `src-tauri` test helpers that build `DocumentInfo`/`Settings` literals, if repeated (a `Default`-based builder or `..Default::default()` where the type allows it).
- A short note in MEMORY.md-worthy report: "new DTO field → update `ui/src/lib/test/fixtures.ts` only".

## Tests
No behavior change: the full suite passes unchanged in count.

`just check` must pass.

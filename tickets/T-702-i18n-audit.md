# T-702 — i18n audit

- **Tier:** Haiku (mechanical; no review loop)
- **Depends on:** the feature work merged so far; run when few UI tickets are in flight.
- **Read first:** CLAUDE.md ("every user-facing string goes through i18n keys"), MEMORY.md (i18n notes: module/param/group keys added by hand, runtime-built keys via `tDynamic`, pre-anticipated keys in en.json), ui/src/lib/i18n/*, ui/src/lib/i18n/en.json.

## Goal
No hard-coded user-facing text in the UI, no missing or unused keys.

## Scope (in)
- A Vitest "i18n lint": every key used via `t("…")` exists in en.json; every `tDynamic` prefix has its keys; report (not fail) unused keys, then delete the ones confirmed unused.
- Find hard-coded user-facing strings in `.svelte` templates/aria-labels/titles and replace them with keys (tests assert via `data-testid`, so text changes are safe).
- Backend notice/error keys: every `key` string produced in src-tauri / crates (`error.*`, `notice.*`) exists in en.json (a Rust test scanning the source for `"error.` / `"notice.` literals vs en.json).
- Placeholder consistency: `{name}` params used in code match the key's text.

## Out
Translations to other languages.

`just check` must pass.

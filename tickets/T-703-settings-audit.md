# T-703 — Settings audit

- **Tier:** Sonnet (no review loop; blocking findings only)
- **Read first:**
  - CLAUDE.md, MEMORY.md (every entry that added a `Settings` field);
  - specs/SPEC-003 §3 and every spec section that defines a setting (`grep -il "settings" specs/*.md`);
  - `src-tauri/src/settings.rs`, `ui/src/lib/state/settings.svelte.ts`, `ui/src/lib/preferences/*`, `ui/src/lib/test/fixtures.ts`.

## Goal
Every setting the specs define exists, has the right default, round-trips, can be reached in the UI where the spec says, and is documented. No orphan settings.

## Scope (in)
1. **Audit matrix.** For each setting, record:
   - the spec reference, the Rust field, the default (spec vs code);
   - migration and fallback: an old file without the field, and an invalid value;
   - where the UI exposes it (Preferences section, View menu, or none) versus where the spec says it should be;
   - whether changing it applies live or at restart, as the spec says;
   - its i18n label and help text.
2. **Fix the gaps:** wrong defaults, missing fallbacks, settings the spec says belong in Preferences but aren't there, and settings in code that no spec mentions (document them in the spec or remove them).
3. **Preferences dialog:**
   - consistent grouping and order: Audio, Recording, Editing, Display/Appearance, Plugins, Advanced;
   - search or filter if the spec asks for it;
   - Reset to defaults, per section and for everything, with a confirmation that uses the H-26 Dialog actions;
   - everything keyboard-accessible.
4. **Settings file robustness:**
   - a corrupt file → back it up and use defaults, with a notice (verify it already works);
   - unknown fields are preserved (T-104);
   - concurrent writes are atomic.

## Tests
- A Rust test that fails if any `Settings` field lacks a default/fallback test, or is missing from `fixtures.ts`.
- One test per fixed gap.
- Preferences reset tests.

`just check` must pass.

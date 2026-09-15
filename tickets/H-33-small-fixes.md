# H-33 — Small fixes found by H-22

- **Tier:** Sonnet (no review loop)
- **Read first:** CLAUDE.md, MEMORY.md (the H-22 and T-709 entries), `crates/presets` (`sanitize_preset_name`, `list_json_names`, `atomic.rs`), `ui/src/lib/tour/WelcomeOffer.svelte` and its test.

## Scope (in)
1. **Preset names.** `sanitize_preset_name` can produce a name that starts with `.`: for example, `"../x"` becomes `".._x"`. `list_json_names` then hides it as if it were an atomic-write temp file, so the preset is saved but never listed.
   - Sanitised names must never start with `.`. Replace or strip leading dots, and keep the result non-empty and unique.
   - Temp-file detection must match only the real temp-name pattern that `atomic.rs` writes, not any leading dot.
   - Add tests: traversal-like names, names made only of dots, and a round trip of save → list → load.
2. **Flaky WelcomeOffer test.** `WelcomeOffer.test.ts` failed once in a full `vitest` run, but passes alone and on a re-run.
   - Find the shared-state or timing cause: module-level stores leaking between test files, fake timers, the recovery `checked` flag, or `localStorage`.
   - Fix it properly, for example by resetting the stores in `beforeEach` or awaiting the right tick.
   - Run the full UI suite three times in a row to show it's stable.

`just check` must pass.

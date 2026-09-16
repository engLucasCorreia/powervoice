# H-69 — Case-insensitive filename collisions break the Windows and macOS build

- **Tier:** Sonnet (mechanical rename plus a guard; blocking findings only)
- **Found by:** H-61, on real CI. Both new jobs fail at `npm run build` with 20 `MISSING_EXPORT` errors before packaging even starts.
- **Cause:** two rune modules sit next to components whose names differ only by case, and every import drops the extension:
  - `ui/src/lib/menu/menubar.svelte.ts` vs `ui/src/lib/menu/MenuBar.svelte`
  - `ui/src/lib/help/shortcutsDialog.svelte.ts` vs `ui/src/lib/help/ShortcutsDialog.svelte`

  On Linux's case-sensitive filesystem the import `"…/menu/menubar.svelte"` is unambiguous. On the default case-insensitive NTFS and APFS the bundler resolves the **component** instead of the state module, so its named exports vanish. The orchestrator confirmed these are the only two collisions in `ui/src` today.

## Scope (in)
1. Rename so no two paths collide case-insensitively. Prefer renaming the **rune modules** (for example `menubarState.ts`, `shortcutsDialogState.ts`) over the components, and follow whatever convention the rest of `ui/src` already uses for `*.svelte.ts` state modules; if most of them would collide under that convention, say so and propose the consistent alternative instead of inventing a third style.
2. Update every import site (~15) plus any test or preview references.
3. **Add a guard to `just check`**: a small script (python stdlib) that fails if any two tracked paths differ only by case, so this can never come back. Unit-test the comparison logic.
4. Confirm on real CI: push the branch and run the release workflow manually (`gh workflow run release.yml --ref ticket/H-69`); the Windows and macOS jobs must get past `npm run build`. Report how far each one then gets — producing an installer is a bonus, not a requirement, since `check_bundle_windows.py`'s extraction path has never run for real.

## Tests
- The collision guard (unit tests plus a run over the real tree).
- The existing UI suites stay green; no behaviour change.

`just check` must pass.

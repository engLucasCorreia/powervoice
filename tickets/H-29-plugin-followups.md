# H-29 — Plugin follow-ups (after T-809)

- **Tier:** Sonnet (no review loop; blocking findings only)
- **Read first:**
  - CLAUDE.md;
  - MEMORY.md (the T-803, T-804, T-809 and H-18 entries);
  - docs/adr/ADR-006 Amendment 1 and ADR-008 Amendments 3–4;
  - `crates/plugin-host/src/{install,catalog,scan}.rs`, `ui/src/lib/plugins/*`, `ui/src/lib/test/fixtures.ts`.

## Scope (in)
1. **Uninstall.** Add an "Uninstall…" action in the Plugin Manager's ⋯ menu, for files inside the per-user install folder only.
   - The user confirms first, using the H-26 Dialog actions with a destructive role.
   - The file is removed, and the plugin leaves the registry and the cache.
   - Open documents that use the plugin keep their slot, shown with the existing "Missing"/failed status, and their state blob is preserved in the sidecar.
   - Files outside the install folder are never touched. They offer "Block" instead.
2. **Duplicate plugin ids.** Use one deterministic policy everywhere: install, rescan and full rescan.
   - Recommended: a user-installed file wins over the standard paths, which win over custom folders; ties go to path order.
   - Document it in an ADR-008 amendment. The manager shows the shadowed duplicate with a "Shadowed by …" note.
3. **Fixtures.** Move the plugin DTO fixtures from `ui/src/lib/plugins/testing.ts` into `ui/src/lib/test/fixtures.ts` (the H-18 pattern) and update the tests that use them.

## Tests
- Uninstall: removes the file, cache and registry entry; refuses files outside the install folder; an open document keeps a Missing slot with its state.
- The duplicate policy, across install and rescan orderings.
- The fixture move changes no test counts.

`just check` and `just check-cross` must pass.

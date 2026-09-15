# H-39 — Loop in the menus, plus i18n cleanup

- **Tier:** Haiku (small, mechanical)
- **Read first:** CLAUDE.md, MEMORY.md (the H-37, T-701, T-702, H-26 and T-709 entries), `ui/src/lib/layout/ViewMenu.svelte` (or wherever transport items live), `ui/src/lib/shortcuts/registry.ts`, `ui/src/lib/tour/tours.ts`, `ui/src/lib/i18n/i18n.test.ts`.

## Scope (in)
1. **Loop Playback menu item.** Add a Loop Playback `menuitemcheckbox` to the menu where playback commands live (View, or a Transport section if one exists).
   - It uses the shared Menu and shows the registry's Ctrl/⌘+L chip.
   - It is bound to the transport store's `loop_enabled`, and disabled without a document.
   - Add a test.
2. **Tour.** Mention looping (the Loop button and Ctrl/⌘+L) in the Welcome tour's playback step, as body text only, with no new anchor. Bump that tour's `version` only if the step text change is significant; if it's just one sentence, leave it.
3. **Unused i18n keys.** Run the unused-key report from `i18n.test.ts`. For each key it lists, delete it only after confirming it has no dynamic use: grep for `tDynamic` prefixes, keys built with template strings, and backend-produced keys. List the kept-but-unused keys in your report.

`just check` must pass.

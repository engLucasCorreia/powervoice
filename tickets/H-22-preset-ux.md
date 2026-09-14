# H-22 — Preset UX follow-ups (T-406)

- **Tier:** Sonnet (no review loop; blocking findings only)
- **Depends on:** T-406 (presets crate `vox-presets`, 11 preset commands, slot/rack preset submenus), H-19 (menu library).
- **Read first:** CLAUDE.md, MEMORY.md (T-406 notes; H-19 menu rows must be direct children of the popup; H-15 dialog patterns), crates/presets, src-tauri/src/ipc/preset_commands.rs, ui/src/lib/rack/{RackSlot.svelte,EffectsMenu.svelte,rack.svelte.ts}.

## Scope (in)
- **Rename** module and rack presets (backend already supports it) via a small name dialog from the menus.
- **Overwrite-confirm on save:** saving under an existing name asks "Replace preset ‹name›?" and uses the existing `overwrite` flag.
- **Manage Presets… dialog:** list user presets per module and rack presets with rename/delete (keyboard accessible); factory presets shown read-only.
- **Import/Export:** export a preset to a `.json` file and import one (validated with the same schema/sanitisation; unknown module ids → placeholder notice).

## Tests
Rename round trip, overwrite confirm/cancel, manage dialog keyboard flow, import rejects traversal/malformed files.

`just check` must pass.

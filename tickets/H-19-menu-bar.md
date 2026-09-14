# H-19 — Real menu bar

- **Tier:** Sonnet (no review loop; blocking findings only)
- **Depends on:** T-209 (File dialogs) and H-16 (View menu) merged — they add menu entries this ticket reorganizes.
- **Read first:** CLAUDE.md, MEMORY.md (H-16 note: `DocumentMenu`/`EditMenu`/`EffectsMenu`/`ViewMenu` are flat always-visible toolbars — no dropdown component exists; keymap registry + `bindingIdentity`), PROMPT.md §3.6 (shortcuts), the Audition-compatible shortcut list in the specs, ui/src/lib/{document,edit,rack,layout,keymap}/*, ui/src/App.svelte, ui/src/lib/theme/tokens.css.

## Goal
A familiar desktop menu bar: File · Edit · View · Effects · Help, each a dropdown with keyboard access and the shortcut shown next to each item, replacing the always-visible button rows.

## Scope (in)
- A reusable `MenuBar` + `Menu` + `MenuItem` component set (ARIA `menubar`/`menu`/`menuitem`, arrow-key navigation, Esc closes, Alt+letter mnemonics, disabled items, separators, checkable items for View toggles).
- Move every existing action into the right menu (File: New Recording, Open, Recent Files ▸, Save, Save As, Export, Recovery & Storage, Close; Edit: Undo/Redo with labels, Cut/Copy/Paste/Delete/Trim/Silence, Select All, Markers ▸; View: Spectral, Analyzer, zoom; Effects: Normalize…, Normalize (LUFS)…, Capture Noise Print, favorites ▸; Help: About with version).
- Shortcut labels come from the keymap registry (one source of truth, platform-aware ⌘/Ctrl).
- Keep the toolbar for transport/record and the favorite normalize buttons.

## Tests
Keyboard navigation and ARIA roles (Vitest), each menu item dispatches the same action as its shortcut, disabled state follows document/recording state, shortcut labels match the registry.

## Out
Native OS menus (Tauri menu API) — a later option; this ticket keeps an in-window menu bar that works identically on all platforms.

`just check` must pass.

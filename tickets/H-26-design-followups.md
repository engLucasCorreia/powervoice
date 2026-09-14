# H-26 — Design follow-ups (after H-25)

- **Tier:** Opus — acts as the `ui-ux-designer` role (`.claude/agents/ui-ux-designer.md`).
- **Depends on:** H-25 (merged). T-803 is merging soon and adds a "Plugins (CLAP)" group to `AddModuleMenu` and a "Loading…" slot status. Migrate `AddModuleMenu` last, after the orchestrator tells you to merge `main`.
- **Read first:**
  - CLAUDE.md;
  - MEMORY.md (H-24 and H-25 notes; A-017 and A-018);
  - `docs/design/design-system.md` and `docs/design/ui-audit.md`;
  - `ui/src/lib/ui/*`, `ui/src/lib/layout/*`, `ui/src/lib/analyzer*`.
- **Live screenshot** after H-25 (owner's running app, 1167×1326): `/tmp/claude-1000/-home-lucas-Documents-dev-audition/e6923653-879f-4025-bf4c-c0fb63498c26/scratchpad/powervoice-ui-after-h25c.png`.

## Scope (in)
1. **Dialog button order per platform.**
   - Windows: primary first (left). macOS and Linux: primary last (right).
   - Put this in one place: a `Dialog` footer helper plus a `platform` util that tests can inject.
   - Use it in every dialog.
2. **One shared menu/popover component** (`ui/src/lib/ui/Menu.svelte` + `Popover`) for every menu:
   - the menu bar menus;
   - Normalize ▾;
   - AddModuleMenu;
   - rack slot menus;
   - preset menus;
   - any other dropdown.

   Requirements:
   - ARIA `menu` / `menuitem`, including `menuitemcheckbox` and `menuitemradio`.
   - Keyboard: arrows, Home/End, typeahead, Enter/Space, Esc returns focus to the trigger, and Left/Right between menu-bar menus.
   - Submenus and separators; right-aligned shortcut chips using `Kbd`; disabled items.
   - Positioning stays inside the viewport (flip or shift).
   - A click outside closes the menu.
3. **Migrate `lucide-svelte`** (deprecated) to `@lucide/svelte`.
   - Keep the `icons.ts` indirection.
   - Run `just notices`.
4. **Unicode minus (U+2212)** in every numeric axis/readout label.
   - Covers analyzer dB, amplitude axis, EQ gain axis, meters and loudness readouts.
   - Use one formatter in `ui/src/lib/ui/units.ts`.
   - Parsers in `NumberField` must still accept an ASCII `-`.
5. **Analyzer unit label overlap.** In the live screenshot, the analyzer's "dBFS" unit label sits on top of the −20 tick.
   - Give unit labels a reserved slot (axis title or corner) so no tick label ever collides.
   - Apply the same check to every axis: time ruler, amplitude, frequency, EQ gain and meter scale.
6. **Screens with real content.**
   - Extend the `?preview` dev page (fixtures only) to show:
     - a document open, in waveform view and in spectral view;
     - while recording (record head, red state, time display, remaining disk);
     - a rack with 4 modules, including an EQ card with its graph;
     - loudness/ACX results;
     - each dialog open.
   - Nothing may overlap or clip at 1280×720 or at 2126×850.
   - Fix what you find.
   - The orchestrator screenshots the owner's running app after the merge. Do NOT start or kill the owner's app.

## Tests
- `Menu`: roles, keyboard, focus return, viewport clamping maths.
- Platform button order.
- The minus formatter, plus a parser round trip.
- Axis-label collision maths: a pure function that is tested.
- Existing tests stay green; update any of them deliberately.

`just check` must pass.

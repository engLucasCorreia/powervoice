# H-25 — Visual design overhaul: big-tech-quality UI (owner request)

- **Tier:** Opus — `ui-ux-designer` agent (`.claude/agents/ui-ux-designer.md`).
- **Owner request (2026-09-14):** "create a UI UX designer specialist agent to fix the UI and make it look good like a big tech app." Earlier the same day: "The spectrum analyzer is very good, and the meters that jump when I play are also very good."
- **Depends on:** H-24 (resizable layout + scales) for phase 2. Phase 1 runs in parallel with H-24 in NEW files only.
- **Screenshot of the current app:** `/tmp/claude-1000/-home-lucas-Documents-dev-audition/e6923653-879f-4025-bf4c-c0fb63498c26/scratchpad/powervoice-ui.png`.
- **Decisions (A-017):** icon set = Lucide via the `lucide-svelte` npm package (ISC; run `just notices` after adding); typography = system font stack (no bundled font).

## Phase 1 (now, new files only — H-24 is editing the layout concurrently)
1. **Design audit:** `docs/design/ui-audit.md` — every screen/panel/dialog from the screenshot and the code: what's wrong (hierarchy, spacing, alignment, contrast, states, units, consistency), prioritised.
2. **Design system spec:** `docs/design/design-system.md` — colour roles (dark + light), type scale, spacing grid, radii, elevation, motion, focus, iconography rules, control sizes, panel anatomy, toolbar grouping, examples.
3. **Tokens:** a new `ui/src/lib/theme/design-tokens.css` (roles + scales, dark default, light via the existing theme mechanism) that the existing `tokens.css` names can map onto later (no breaking renames in phase 1).
4. **Component kit** in `ui/src/lib/ui/`: `Button` (primary/secondary/ghost/danger, sizes), `IconButton` (with tooltip), `ToggleButton`/`SegmentedControl`, `Select`, `Slider` + `NumberField` with units, `Toggle` (switch), `PanelHeader` (title, actions, collapse), `Tabs`, `Tooltip`, `Badge`/`StatusDot`, `Kbd` (shortcut chip), `Separator`, `EmptyState`, `Icon` (Lucide wrapper). Each with Vitest tests (ARIA, states, keyboard) and a `ui/src/lib/ui/Gallery.svelte` dev page (mounted only behind a dev flag) showing every component in every state.

## Phase 2 (after H-24 merges — the orchestrator will tell you to merge main)
5. Apply the system everywhere: menu bar, toolbar → grouped transport bar (icon buttons with tooltips for Play/Stop/Record/Return to start/Loop; time display in tabular figures; recording state), record panel, markers panel, rack panel + slot cards + EQ graph chrome, loudness/ACX panel, analyzer + meter bridge chrome (keep their rendering and look), editor view chrome (rulers, scrollbar, divider), all dialogs (consistent layout, primary/secondary button order per platform), notices/toasts, preferences.
6. Empty states (no document: a welcoming "Open a file or press Record" state with shortcuts), loading/progress states, focus/hover/active/disabled everywhere.
7. Light theme parity; contrast checks (AA) as tests where feasible.
8. Screenshot-ready: nothing overlaps or clips at 1280×720 and at the owner's 2126×850 window.

## Tests
Component kit tests; per-screen structure tests stay green (update deliberately); a contrast test for token pairs (compute WCAG ratios for text/background roles in both themes).

`just check` must pass.

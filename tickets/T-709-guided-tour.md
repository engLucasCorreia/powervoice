# T-709 — Guided Tour widget (owner request)

- **Tier:** Opus, in the `ui-ux-designer` role (`.claude/agents/ui-ux-designer.md`).
- **Owner request (2026-09-15):** "add the Tour widget to teach how to use the app."
- **Depends on:** T-708 (the tour must look right in every theme) and T-809 (the UI is stable).
- **Read first:**
  - CLAUDE.md, MEMORY.md (H-25 and H-26 entries: the kit, `Popover` placement maths, `Dialog`, i18n);
  - `docs/design/design-system.md`;
  - `docs/user-guide.md` (the tour follows its first-recording flow);
  - `ui/src/App.svelte` and the panel components.

## Scope (in)
1. **A Tour engine** in `ui/src/lib/tour/`, with no new dependency:
   - A spotlight overlay: a dimmed backdrop with a cut-out around the target and a rounded highlight that follows resize, scroll and panel splitters.
   - An anchored step card built on H-26's placement maths: it flips and shifts to stay in view.
   - Title, body, an optional illustration, "Step n of N", Back/Next/Skip/Done.
   - Keyboard: ←/→, Enter, Esc to skip. Focus moves to the card, and every step is announced to screen readers (`aria-live`).
   - Respects `prefers-reduced-motion`.
   - Steps target elements by `data-tour="…"` attributes. A missing target skips gracefully to a centred card.
   - Steps can wait for an action: "Click Record to continue" advances when the app state changes.
2. **Tours (content through i18n keys):**
   - **Welcome tour** (about 8–10 steps): the layout; choosing the input and output device; Record (with a practice take) and the time display; waveform navigation (zoom/scroll) and the spectral view; markers; the effects rack (Add module, EQ); noise reduction; meters, the analyzer and loudness/ACX; Normalize favourites; export; where to find help and how to replay the tour.
   - **Short contextual tours**, each started from a "?" in its panel header: the rack and presets; noise print and reduction; loudness/ACX and normalize; punch-in; the Plugin Manager.
3. **When the tour appears:**
   - The Welcome tour is offered on first run with Start / Later / Don't show again, but never over the crash-recovery dialog or while recording.
   - Help → "Take the Tour" and Help → "Tours ▸" (`Menu`) replay any tour at any time.
   - Progress is stored in an additive `Settings.tours` field (completed or dismissed ids, versioned so new steps can re-offer a tour).
   - Update `ui/src/lib/test/fixtures.ts` (H-18) for the new field.
4. **Preview:** a `?preview&scene=tour&step=n` scene, screenshot-clean in all themes at 1280×720 and 2126×850.

## Tests
- Engine: next/back/skip, keyboard, a missing target, placement flip, an action-gated step, resize tracking, reduced motion.
- First-run offer logic: not over recovery, not while recording, "Don't show again" persists.
- Every step's target exists in the rendered app shell. The test fails if someone removes a `data-tour` anchor.

`just check` must pass.

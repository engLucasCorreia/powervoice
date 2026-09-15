# T-708 — Themes: first-class Light ("clear") mode, System, High contrast, everywhere (owner request)

- **Tier:** Opus, in the `ui-ux-designer` role (`.claude/agents/ui-ux-designer.md`).
- **Owner request (2026-09-15):** "add themes to the app including not only dark mode, but a clear mode as well."
- **State today (H-25):**
  - `Settings.theme` = `dark | light | system`, switched in Preferences → Appearance;
  - tokens in `ui/src/lib/theme/design-tokens.css`, `theme-bridge.css` and `theme.svelte.ts`;
  - AA contrast tests.
  - Canvas and WebGL renderers still hard-code colours in places (`ui/src/lib/render/quads.ts`, `ui/src/lib/spectrogram/colormap.ts`, and possibly the analyzer, meters and EQ graph).
  - Light was never verified screen by screen.
- **Depends on:** T-809 and H-28 (they are editing UI now). Dispatch when they have merged.
- **Read first:**
  - CLAUDE.md, MEMORY.md (H-25 and H-26 entries; A-017 and A-018: dark is the default);
  - `docs/design/design-system.md` §15;
  - `ui/src/lib/theme/*`, every renderer in `ui/src/lib/{waveform,spectrogram,analyzer,meters,eq,render}`.

## Scope (in)
1. **Light ("clear") theme at full parity.**
   - Every screen, panel, dialog, menu, toast and state is designed for light, not just inverted: waveform, selection, playhead/record head, markers, rulers, spectrogram colormap choice, analyzer, meters (keep their "jump" feel), EQ graph, gain-reduction meters, loudness/ACX, rack cards, empty states and the Plugin Manager.
   - Canvas and WebGL renderers read their colours from theme tokens: a typed `themeColors()` read from CSS custom properties, cached, and invalidated on theme change. Switching theme repaints them live without a reload.
2. **Themes:** Dark (default), Light, Match System (live `prefers-color-scheme`) and High Contrast (dark-based, WCAG AAA text, thicker focus rings and waveform strokes).
   - The theme system is data-driven (one token block per theme) so more themes can be added later.
3. **Switching:** Preferences → Appearance, with a live preview swatch per theme, plus View → Theme ▸ (the H-26 `Menu` radio items).
   - A shortcut only if SPEC-003 has one free; don't invent a conflicting one.
   - Before the UI mounts, apply the saved theme so there's no flash of the wrong one.
4. **Spectrogram colormaps:** keep the scientific colormaps. The default per theme stays readable, and the user's explicit colormap choice wins.
5. **Verification:**
   - `?preview` scenes (from H-26) take `&theme=`.
   - Screenshots of every scene in all 4 themes at 1280×720 and 2126×850. Nothing unreadable, nothing hard-coded.
   - The contrast tests cover all 4 themes' token pairs.
   - A test fails if a component or renderer uses a raw colour literal outside the token files (a lint-style scan with an allowlist for colormaps).

## Tests
- Theme resolution (including system changes live).
- Renderer colour invalidation.
- Contrast for each theme.
- The raw-colour scan.
- Menu and Preferences switching.

`just check` must pass.

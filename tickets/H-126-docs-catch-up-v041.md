# H-126 — Documentation sweep for everything shipped in v0.4.1

- **Tier:** Sonnet
- **Why now:** development pauses after this. The docs, the in-app Help Centre and the guided tour must describe the app as it is at v0.4.1, so anyone (including a future session) can pick it up without reading the ticket history.

- **Read first:** CLAUDE.md, `docs/README.md`, `docs/user-guide.md`, `docs/faq.md`, `docs/shortcuts.md`, `ui/src/lib/help/`, `ui/src/lib/tour/tours.ts`, and the v0.4.1 release notes (`gh release view v0.4.1`).

## Scope (in)
Check, and fix where wrong or missing, the user-facing documentation for everything added or changed since v0.4.0 (tickets H-108..H-125 — read their board rows and ticket files):
1. **Explain My Voice**: progress and Cancel, the big/maximisable window, the Annotations toggle, EQ Advice saying when it has nothing to suggest, and the **export** (image + HTML report) with where the file goes.
2. **Spectrum Inspector**: the curve legend (dashed gray = room tone) and its own Explain button.
3. **EQ**: widening the rack, the cursor readout, node hover readouts, the right-click menu (add/delete/enable/reset/slope), double-click to expand.
4. **Meters**: the vertical input meter, the −60/−80/−120 dBFS floor, and the new Fast/Medium/Slow **Speed** control — say what each speed does.
5. **Recording**: the waveform now grows live (remove any text that says otherwise).
6. **Spectral view**: the selection tint and the time/Hz/dB readout (hidden while dragging).
7. **Installing the AppImage**: `chmod +x` before running — the owner hit "There is no app installed for AppImage application bundle" on 2026-09-22. Make sure `docs/` and the FAQ say this, not just the README.
8. Screenshots in `docs/images/` that are now wrong (e.g. showing a gray selection): list them in your report. Do not fake new ones; say which need retaking.

Keep the existing tone. Every user-facing string still goes through i18n. Run `just help-content` so the in-app Help Centre matches, and check the guided tour still describes what the app does.

## Tests
`just check` must pass (it checks help content, docs and notices). Add or update tests only where the docs checks need them.

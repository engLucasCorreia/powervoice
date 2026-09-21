# H-107 — A Help section inside the app (owner request)

- **Tier:** Sonnet
- **Owner's words:** "I hope you created the 'help' section, a wiki, how to install the third party plugins, how to use it, etc."
- **The state today:** Help offers **About**, **Tours** and **Keyboard Shortcuts** — and nothing else. Everything else we have written lives in `docs/` on GitHub, which is no use to someone inside the app wondering how to install a VST3.
- **Read first:** CLAUDE.md, MEMORY.md, `ui/src/lib/help/` (the existing dialogs and their shape), `ui/src/lib/layout/HelpMenu.svelte`, `docs/user-guide.md` and `docs/faq.md` (the content already exists — this ticket is about getting it in front of the user, not rewriting it), specs/SPEC-019 if it covers Help, and T-709's tour work.

## Scope (in)
1. **A Help centre inside the app**: browsable, searchable, offline. It must not be a link to a website — a user who cannot get audio working may not be online, and a link is not help.
2. **The content it must cover**, drawn from the existing docs so there is one source of truth rather than two that drift:
   - getting started: open or record, edit, export;
   - **installing third-party plugins** — CLAP, VST3, LV2, JSFX: where they live on each platform, how to scan, what the Plugin Manager's states mean, what to do when one is blocklisted, and that LV2 needs the system `lilv` library;
   - the effects rack and what each module is for;
   - noise reduction, loudness and ACX, exporting;
   - Explain My Voice and how to read it;
   - troubleshooting: no devices, no audio, MP3 export unavailable, plugin windows on Wayland.
3. **Reachable the obvious ways**: the Help menu, **F1**, and a "?" from the panels where someone gets stuck (the Plugin Manager especially).
4. Decide how the content is kept in sync with `docs/` and say why — generated at build time from the Markdown is the obvious answer; a hand-maintained copy that drifts is the failure to avoid.

## Out
Rewriting the documentation itself (H-105 does that), and any external wiki.

## Verification
Screenshots of the Help centre in Dark and Light, desktop and phone, including the plugin-installation page, into the scratchpad `h107/`. Check search finds a term that only appears in the body text.

`just check` must pass.

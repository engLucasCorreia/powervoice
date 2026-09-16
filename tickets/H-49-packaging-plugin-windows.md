# H-49 — Packaging and docs for plugin windows

- **Tier:** Haiku (small, mechanical)
- **Found by:** T-901 (plugin editor windows).
- **Read first:** CLAUDE.md, MEMORY.md (the T-901, H-38, H-45 and T-807 entries; decisions A-022 and A-027), `src-tauri/tauri.conf.json` (bundle → linux → deb `recommends`), `docs/building.md`, `docs/user-guide.md`, `crates/sandbox/src/gui/` (which libraries are loaded at runtime, and their exact sonames).

## Scope (in)
1. **deb recommends:** add the packages that plugin windows load at runtime — `libsuil-0-0` (LV2 plugin UIs) and the libX11 runtime package if it isn't already pulled in by the WebView dependencies. Check the sonames the code actually dlopens before naming packages; don't add hard dependencies.
2. **docs/building.md:** a short section on the runtime libraries plugin windows need per platform, and what happens without them (the window button is disabled with a reason).
3. **docs/user-guide.md:** a plain-language note in the plugins section:
   - a plugin's own window opens from its rack slot;
   - on Linux it needs X11 or XWayland, and LV2 windows need suil;
   - Windows is untested and macOS isn't supported yet;
   - after a plugin crash the window doesn't reopen by itself (A-027).

`just check` must pass.

# H-38 — Packaging: lilv as a recommended dependency (A-022)

- **Tier:** Haiku (mechanical)
- **Read first:** CLAUDE.md, MEMORY.md (T-807 and T-705 entries; A-022), `src-tauri/tauri.conf.json` (the bundle section: deb `depends`), `docs/building.md`, `docs/user-guide.md` (plugins section), any packaging files (AppImage, Arch PKGBUILD if present).

## Scope (in)
1. **deb:** add `liblilv-0-0` as a *Recommends*, not a Depends. Use Tauri's deb `recommends` field if the pinned Tauri version supports it. Otherwise document the limitation and leave `depends` unchanged; don't make lilv a hard dependency.
2. **Arch:** if a PKGBUILD exists, add `optdepends=('lilv: LV2 plugin support')`; if none exists, document it in `docs/building.md`.
3. **AppImage/other:** a note in `docs/building.md` that LV2 needs the system lilv (`POWERVOICE_LILV` overrides the library path).
4. **User guide:** a short plain-language note in the plugins section saying that LV2 plugins need the "lilv" library, with how to install it on Debian/Ubuntu, Fedora and Arch, and the in-app message you see when it's missing.

`just check` must pass.

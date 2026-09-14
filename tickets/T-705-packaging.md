# T-705 — Packaging: Linux bundles, Windows/macOS build docs, README + user guide

- **Tier:** Sonnet (no review loop; blocking findings only)
- **Depends on:** the vertical slices + hardening (done); T-701..T-704 audits may land in parallel — packaging must not wait for them.
- **Read first:** CLAUDE.md (no system package installs — if a tool is missing, document it and stop that sub-step), PROMPT.md §2 (LOCKED decisions: Tauri 2, MIT OR Apache-2.0, cross-platform), docs/adr/ADR-007-licensing.md (+ amendments: LAME loaded at runtime, never bundled; symphonia MPL-2.0; third-party notices), ADR-009 Amendment 1 (Linux WebKit DMA-BUF default), MEMORY.md (D-016 WebKit flag, H-11 `windows-sys`, `just check-cross`), justfile (`build`, `check-cross`), src-tauri/tauri.conf.json, ui/README.md.

## Goal
The owner can install PowerVoice like a normal app: a Linux AppImage and .deb built with `just build`, and clear, tested-as-far-as-possible instructions to build the Windows installer and macOS app.

## Scope (in)
- **Tauri bundle config:** product name, identifier (ADR-004's `app.powervoice.editor` unless ADR-005's open question says otherwise — report), version from the workspace, icons (generate a simple placeholder icon set from an SVG if none exists), categories (AudioVideo;Audio), file associations for .wav/.flac/.mp3/.m4a/.ogg (Open with PowerVoice), Linux desktop entry, deb dependencies (webkit2gtk, libasound2; LAME optional/recommended, not required).
- **Linux:** `just build` produces AppImage + .deb in `target/release/bundle/`; the WebKit DMA-BUF default (ADR-009 Amendment 1) applies inside the packaged app (already in `webkit.rs` — verify). If `appimagetool`/`dpkg-deb` aren't available, build what's possible and document the missing tools (no sudo).
- **Windows/macOS:** `docs/building.md` with exact prerequisites and commands (MSVC/WebView2 for Windows, Xcode CLT for macOS), code-signing notes (unsigned by default), where LAME comes from on each OS; `just check-cross` stays green.
- **Licensing:** `THIRD_PARTY_NOTICES` generated from the dependency tree (cargo metadata + npm licenses, no new tool dependency — a small script), shown in Help → About; LICENSE-MIT / LICENSE-APACHE at the repo root if missing.
- **README.md + docs/user-guide.md:** what PowerVoice is, install, first recording, cleanup chain, presets, ACX export, shortcuts table (from the keymap registry), troubleshooting (audio devices on PipeWire/JACK/ALSA, WebKit rendering flag, MP3 needs LAME).

## Tests
`just build` succeeds (report bundle paths + sizes); a script test that the notices file lists every direct dependency; the desktop entry validates (`desktop-file-validate` if available, else a format test).

## Out
Code signing/notarization, auto-update, Flatpak/Snap, store submissions.

`just check` must pass.

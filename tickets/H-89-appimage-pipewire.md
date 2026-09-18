# H-89 — The AppImage bundles libpipewire and breaks outside Ubuntu (owner-reported, release blocker)

- **Tier:** Sonnet. **This is shipped-and-broken**, found by the owner installing v0.3.0 on Arch (Omarchy).

## The bug, already diagnosed — confirm, don't re-derive
`PowerVoice_0.3.0_amd64.AppImage` contains `usr/lib/libpipewire-0.3.so.0` but **not** PipeWire's
`spa-0.2` plugin directory. The bundled library was built on Ubuntu, where those plugins live at
`/usr/lib/x86_64-linux-gnu/spa-0.2`; on Arch they are at `/usr/lib/spa-0.2`. So the app loads the
bundled library, which then can't find its own plugins, and **retries in a tight loop**, flooding
stderr with `[E] pw.loop … can't make support.system handle: No such file or directory`.

Confirmed two ways on the owner's machine:
- deleting `usr/lib/libpipewire-0.3.so.0` from an extracted copy → zero PipeWire errors, app runs;
- `LD_PRELOAD=/usr/lib/libpipewire-0.3.so.0` on the shipped AppImage → same.

This breaks **every non-Ubuntu distribution** (Arch, Fedora, openSUSE…), i.e. most AppImage users.
`libpipewire` is the wrong class of library to bundle at all: like graphics drivers, it must match
the daemon running on the user's system.

- **Read first:** CLAUDE.md, MEMORY.md (H-61/H-69's packaging work and the `check_bundle.py` guard pattern; the v0.3.0 release entry), `scripts/packaging/tauri_build.sh`, `scripts/packaging/check_bundle.py`, `src-tauri/tauri.conf.json`'s `bundle.linux.appimage`, `.github/workflows/release.yml`.

## Scope (in)
1. **Stop shipping `libpipewire-0.3.so.0` inside the AppImage.** Tauri drives linuxdeploy, which
   copies every `NEEDED` library, so the cleanest hook is a post-bundle step in the packaging
   scripts that removes the library from the AppDir/AppImage and repacks. Whatever mechanism you
   choose, it must run in `just build` **and** in CI, from one source of truth.
2. **Audit the other 160 bundled libraries for the same class of problem** — anything that loads
   its own plugins or must match a system daemon. Report what you find even if you only fix
   libpipewire; `libasound` is already a system dependency rather than bundled, which is the model.
3. **A guard in `check_bundle.py`** so a bundle containing a denylisted library fails the build,
   the way H-69's case-collision check prevents its bug from coming back. This is the part that
   makes the fix permanent.

## Verification (the important part)
You have the broken artifact locally: `/home/lucas/Downloads/PowerVoice_0.3.0_amd64.AppImage`, and
this machine is Arch, so it reproduces. **Do not do a full release rebuild just to test** — repack
that existing AppImage with your fix applied and run it here: zero `pw.loop` errors, and a window
appears. Then confirm your change is wired into the real build path.

If you need `appimagetool`, download it to the session scratchpad — do not install system packages,
and never start or kill the owner's running PowerVoice.

Note: a separate `Could not create surfaceless EGL display: EGL_BAD_ALLOC` message appears on this
Wayland machine. The window renders anyway. Say whether it is benign or a second bug; don't fix it
here unless it is trivially the same root cause.

`just check` must pass.

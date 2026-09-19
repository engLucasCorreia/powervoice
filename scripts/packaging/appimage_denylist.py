#!/usr/bin/env python3
"""Single source of truth: libraries that must never ship inside a bundle (H-89).

Imported by both `strip_appimage_libs.py` (removes them from a built AppImage and repacks it —
the fix) and `check_bundle.py` (fails the build if one slips back in anyway — the guard that makes
the fix permanent, the same pattern H-69's `check_case_collisions.py` established).

**What belongs on this list:** a library that, like a graphics driver, must match something
running on the user's system rather than be vendored — usually because it loads its own plugins
from a directory it looks up at an *absolute, build-time-baked-in path* that differs across
distros, and that plugin directory is not itself relocated/bundled by any of the linuxdeploy
plugins Tauri's bundler already runs. Contrast with the many libraries this AppImage bundles that
*are* safe: e.g. GTK's immodules/printbackends, GDK-pixbuf's loaders and WebKitGTK's injected
bundle/web process all ship *together with* their own plugin directories under
`usr/lib/x86_64-linux-gnu/...`, and `AppRun` (via `apprun-hooks/linuxdeploy-plugin-gtk.sh`) points
their lookup env vars at those bundled copies — self-contained, no host-path dependency. PipeWire
has no such linuxdeploy plugin.

- H-89: `libpipewire-0.3.so.0` — pulled into the AppImage transitively (WebKitGTK's media/WebAudio
  backend needs it), but its `spa-0.2` plugin directory is not: PipeWire looks it up at the
  absolute path it was compiled with (Ubuntu: `/usr/lib/x86_64-linux-gnu/spa-0.2`; Arch:
  `/usr/lib/spa-0.2`), so the bundled library loads, can't find its plugins there, and retries in a
  tight loop, flooding stderr with `[E] pw.loop ... can't make support.system handle`. Confirmed
  on the owner's Arch machine; PowerVoice's own audio I/O never goes through this bundled copy
  (the app talks to PipeWire through `cpal`/`vox-engine`, dynamically linked against the *system*
  libpipewire — same model as `libasound2`/ALSA, already a `.deb` dependency rather than bundled).

- H-90: `libwayland-client.so.0` — the Wayland protocol client. Every Wayland-facing library in
  the process (system Mesa/EGL, the GTK backend, WebKitGTK's renderer) must share **one** copy,
  and it is the compositor's peer, so it belongs to the host exactly as a graphics driver does —
  it is on upstream's canonical never-bundle list (pkg2appimage `excludelist`) for that reason.
  Bundled here (built on Ubuntu) against a newer system stack, EGL failed with `EGL_BAD_ALLOC` and
  **`WebKitWebProcess` aborted on every launch**: the window opened and stayed completely empty,
  with nothing in the log but `Could not create surfaceless EGL display`. Proven on the owner's
  Arch machine by removing this one file from v0.3.1's AppImage and repacking: the full UI renders
  and the renderer stops crashing. The other bundled `libwayland-*` (cursor, egl, server) are
  **not** listed: removing this one alone is sufficient, and the module's own rule is not to
  denylist speculatively.

  **How this was missed in H-89:** that audit looked for libraries whose *plugin directories* were
  missing, which is how PipeWire failed. This one fails differently — a singleton that must match
  the host, with no plugins involved. When auditing a bundle, check it against the upstream
  `excludelist` as well as reasoning about plugin lookup; of the 160 libraries bundled here, that
  list flagged exactly this one.

**Audited but NOT denylisted (H-89), for a future ticket if either turns out to matter:**
- `libenchant-2.so.2` — same shape of risk as PipeWire (its provider-backend plugin directory,
  typically `<libdir>/enchant-2/`, isn't bundled alongside it either), but enchant's providers are
  looked up lazily and it degrades to "no spell-check backends" rather than PipeWire's hard retry
  loop; only reachable if a WebView text field's spell-check is actually exercised. No repro of a
  visible failure.
- `libgstreamer-1.0.so.0` and the other bundled `libgst*`/`liborc-0.4` libraries — bundled without
  a GStreamer plugin directory because `tauri.conf.json`'s `bundle.linux.appimage.bundleMediaFramework`
  is `false` (deliberately, to avoid bundling heavyweight codec plugins). Same absolute-path-lookup
  shape as PipeWire in principle, but these libraries back WebView `<video>`/`<audio>` elements
  PowerVoice's UI doesn't use (playback goes through `cpal`, not the WebView), so degrading to "no
  plugins found" has no observed effect. Confirmed no `gstreamer-1.0` plugin directory is bundled.

Neither is reachable in normal use the way PipeWire's media-session probing apparently is (it
appears to run at WebKitGTK/GTK startup regardless of whether audio is used), so both are left
alone here rather than denylisted speculatively — see H-89's final report for the full audit.
"""

from __future__ import annotations

# Glob patterns (glob syntax, not regex) matched against library *filenames* anywhere under a
# bundle's usr/lib (recursively — so version-suffixed real files like `libpipewire-0.3.so.0.123.0`
# are caught too, not just the unversioned symlink name).
DENYLISTED_LIBS: list[str] = [
    "libpipewire-0.3.so*",
    "libwayland-client.so*",
]

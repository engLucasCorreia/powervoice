# Building PowerVoice

PowerVoice is a [Tauri 2](https://v2.tauri.app/) app: a Rust core (`src-tauri` + the `crates/`
workspace) plus a Svelte 5 / TypeScript UI (`ui/`). This covers building it yourself on every
platform. The product owner develops and tests on **Linux (Arch, PipeWire)** only — Windows and
macOS builds are expected to compile and follow platform-neutral code paths (PROMPT.md §2), but are
**unverified by the owner** until someone actually builds and runs one; treat problems there as
real bugs to report, not confirmed-working paths.

## Common prerequisites (every OS)

- **Rust** (stable, edition 2024) via [rustup](https://rustup.rs/).
- **Node.js** 20+ and npm, for the `ui/` frontend.
- Run `npm ci --prefix ui` once after cloning (and again whenever `ui/package-lock.json` changes).

Then, from the repo root:

| Command | What |
|---|---|
| `just dev` | run the app in dev mode (hot-reloading UI) |
| `just check` | fmt check + clippy (`-D warnings`) + all Rust tests + svelte-check + vitest |
| `just build` | release build (+ the Linux Tauri bundle where applicable) |
| `just check-cross` | `cargo check` the non-Tauri crates for `x86_64-pc-windows-gnu`, without a full Windows toolchain |

(No `just` on your machine? Install it from [github.com/casey/just](https://github.com/casey/just),
or read `justfile` and run the underlying `cargo`/`npm` commands directly.)

## Linux

Tauri 2's WebView on Linux is WebKitGTK. You need its development headers, GTK3's, and ALSA's, to
*build*; end users of a `.deb` only need the runtime libraries (declared as `Depends`/`Recommends`
in `src-tauri/tauri.conf.json`'s `bundle.linux.deb`).

Arch:

```sh
sudo pacman -S webkit2gtk-4.1 gtk3 alsa-lib
```

Debian/Ubuntu (package names per the [Tauri 2 prerequisites guide](https://v2.tauri.app/start/prerequisites/)):

```sh
sudo apt install libwebkit2gtk-4.1-dev build-essential curl wget file libxdo-dev \
    libssl-dev libayatana-appindicator3-dev librsvg2-dev libasound2-dev
```

Fedora: `sudo dnf install webkit2gtk4.1-devel gtk3-devel alsa-lib-devel`.

Then:

```sh
just build
```

produces, under `target/release/bundle/`:

- `appimage/PowerVoice_<version>_amd64.AppImage`
- `deb/PowerVoice_<version>_amd64.deb`
- (also `rpm/PowerVoice-<version>-1.x86_64.rpm`, a side effect of Tauri's default `"targets": "all"`
  — not a ticket requirement, untested on an actual RPM distro)

### The plugin sandbox (H-45)

Third-party plugins (CLAP/VST3/LV2/JSFX) run in a separate `powervoice-sandbox` process, which the
app looks for beside its own executable
(`vox_plugin_host::SandboxOptions::beside_current_exe`). `just build` builds that binary, stages it
at `src-tauri/binaries/powervoice-sandbox-<host-triple>` (`scripts/packaging/build_sandbox.sh`, the
triple from `rustc -vV`), and bundles it as a Tauri external binary
(`scripts/packaging/tauri_build.sh`, passing `externalBin` on the `tauri build` command line
instead of in `src-tauri/tauri.conf.json` — that keeps plain `cargo build`, `just check` and
`just dev` working even when the staged binary doesn't exist yet, since `tauri-build` only
validates `externalBin` paths that are actually declared). The AppImage and `.deb` both end up
with `usr/bin/powervoice-sandbox` next to `usr/bin/powervoice-app`; `just build` checks that with
`scripts/packaging/check_bundle.py` right after building (fails the build if it's missing).
`src-tauri/binaries/` is never committed — it's rebuilt fresh by every `just build`/CI run, for the
host's own triple. The GitHub release workflow (`.github/workflows/release.yml`) uses the same two
scripts, so there's one source of truth for how the sandbox gets staged and bundled.

### If a bundler tool is missing or fails

CLAUDE.md forbids this project's agents from installing system packages, so a ticket run that hits
a missing tool documents it here rather than working around it with `sudo`. As a human building
locally, you're free to install what's missing yourself:

- **`.deb`**: Tauri's bundler writes the `.deb` archive itself (control file + data tarball) — it
  does **not** shell out to the system's `dpkg-deb`, so nothing extra to install here.
- **AppImage**: the bundler downloads `linuxdeploy`, its `gtk`/`gstreamer` plugins, and
  `appimagetool` on first use, into `~/.cache/tauri/` (needs network access; no `sudo`). Two
  environment-specific failure modes were hit while building this ticket on the owner's Arch
  machine, both **upstream `linuxdeploy`/`linuxdeploy-plugin-gtk` assumptions that don't hold on a
  very new/rolling-release distro** — not something PowerVoice's config controls:
  1. `linuxdeploy`'s bundled `strip` (built against an older binutils) doesn't understand the
     `SHT_RELR` (`.relr.dyn`) relocation sections newer system libraries can use, and errors on
     every one of them. Workaround: `NO_STRIP=1 npm --prefix ui run tauri build` (skips stripping
     instead of failing; the resulting binaries/libraries are a little larger, uncompressed size
     only — the AppImage itself is still squashfs-compressed).
  2. `linuxdeploy-plugin-gtk` hard-codes the classic `gdk-pixbuf` loader-module layout
     (`pkg-config --variable=gdk_pixbuf_binarydir gdk-pixbuf-2.0`). Some distros (e.g. current Arch)
     ship a `gdk-pixbuf-2.0` built around GNOME's newer sandboxed `glycin` image decoders instead,
     with no loader-module directory to copy at all, and the plugin's `cp` fails outright. If you
     hit "`cp: cannot stat '.../gdk-pixbuf-2.0/2.10.0': No such file or directory`", point
     `PKG_CONFIG_PATH` at a `.pc` override for `gdk-pixbuf-2.0` whose `gdk_pixbuf_binarydir` names
     an empty directory that does exist, e.g.:
     ```sh
     mkdir -p /tmp/pixbuf-stub/lib/gdk-pixbuf-2.0/2.10.0/loaders
     touch /tmp/pixbuf-stub/lib/gdk-pixbuf-2.0/2.10.0/loaders.cache
     sed -e 's|^libdir=.*|libdir=/tmp/pixbuf-stub/lib|' \
         /usr/lib/pkgconfig/gdk-pixbuf-2.0.pc > /tmp/pixbuf-stub/gdk-pixbuf-2.0.pc
     APPIMAGE_EXTRACT_AND_RUN=1 NO_STRIP=1 PKG_CONFIG_PATH=/tmp/pixbuf-stub \
         npm --prefix ui run tauri build -- --bundles appimage
     ```
     The resulting AppImage simply won't bundle its own copy of GDK pixbuf loaders (it'll rely on
     the target system's, same as it would if this whole plugin step were absent) — harmless for
     PowerVoice, which draws its waveform/spectrogram in the WebView (Canvas2D/WebGL2), not through
     GDK pixbufs.
  3. Building without any FUSE-based AppImage execution at all (a sandboxed CI runner, no
     `/dev/fuse`): set `APPIMAGE_EXTRACT_AND_RUN=1` so the downloaded AppImage-format tools
     (`linuxdeploy`, `appimagetool`) extract themselves and run directly instead of mounting via
     FUSE.
  - If AppImage bundling still doesn't work after that, it's still fine to ship just the `.deb` —
    or build the AppImage in a plain Debian/Ubuntu container/VM, which is the standard advice for
    AppImages anyway (an older glibc baseline maximizes compatibility with older target systems,
    and sidesteps both quirks above since they're specific to newer/rolling distros).
- **Desktop entry validation**: `python3 scripts/packaging/check_desktop_entry.py` (run
  automatically by `just build`) uses `desktop-file-validate` (from `desktop-file-utils`) if it's
  on `PATH`, else falls back to its own format check — no hard requirement either way.

## Windows

1. Install the [Rust MSVC toolchain](https://rustup.rs/) (`stable-x86_64-pc-windows-msvc`).
2. Install the [Visual Studio Build Tools](https://visualstudio.microsoft.com/visual-cpp-build-tools/)
   with the "Desktop development with C++" workload (provides the MSVC linker `link.exe` and the
   Windows SDK).
3. Install the [WebView2 runtime](https://developer.microsoft.com/microsoft-edge/webview2/) —
   pre-installed on current Windows 10/11, but check if you're on an older image; Tauri's WebView
   on Windows is WebView2 (Edge/Chromium), not WebKitGTK.
4. `npm ci --prefix ui`, then `just build` (or `cargo build --workspace --release` +
   `npm --prefix ui run tauri build` directly).

This produces an NSIS installer (`.exe`) and/or an MSI under `target/release/bundle/` (Tauri 2's
default Windows bundle targets). **Unsigned by default** — no code-signing certificate is
configured (out of scope per the ticket); Windows SmartScreen will warn on first run. To sign,
see [Tauri's Windows signing guide](https://v2.tauri.app/distribute/sign/windows/) and set the
`bundle.windows.certificateThumbprint`/`signCommand` config or `TAURI_SIGNING_...` environment
variables — none of that is wired up here.

`libmp3lame` (MP3 export) on Windows: there's no OS-provided package manager equivalent to
`apt`/`pacman`. Download a `libmp3lame.dll` build (e.g. from a trusted static-build distributor
such as [Rareware's LAME Windows builds](https://www.rarewares.org/mp3-lame-bundle.php), or build
LAME yourself from [the upstream source](https://lame.sourceforge.io/)) and place it next to
`powervoice-app.exe`, or anywhere else on the DLL search path. PowerVoice loads it by name at
runtime (`libmp3lame.dll` — ADR-007 §4) and disables MP3 export with an explanatory message if it's
absent; nothing else is affected.

## macOS

1. Install [Xcode Command Line Tools](https://developer.apple.com/xcode/resources/): `xcode-select --install`.
2. Install Rust via [rustup](https://rustup.rs/) (`stable-x86_64-apple-darwin` and/or
   `stable-aarch64-apple-darwin` for Apple Silicon).
3. `npm ci --prefix ui`, then `just build`.

Tauri's WebView on macOS is `WKWebView`/`WebKit.framework` (system-provided — no separate install).
This produces a `.app` bundle and a `.dmg` under `target/release/bundle/`.

**Unsigned/not notarized by default** (out of scope per the ticket). Gatekeeper will refuse to open
an unsigned, unnotarized app downloaded from the internet ("PowerVoice is damaged and can't be
opened" or similar) unless the user right-clicks → Open, or clears the quarantine attribute:
`xattr -cr PowerVoice.app`. To actually sign and notarize for distribution, see
[Tauri's macOS signing guide](https://v2.tauri.app/distribute/sign/macos/) — needs an Apple
Developer account and certificate, not set up here.

`libmp3lame` (MP3 export) on macOS: install via [Homebrew](https://brew.sh/) —
`brew install lame` — which provides `libmp3lame.dylib` (found via the dynamic linker's default
search paths, e.g. `/opt/homebrew/lib` on Apple Silicon or `/usr/local/lib` on Intel). PowerVoice
loads it by name at runtime; MP3 export is disabled with an explanatory message if it's absent.
Note (ADR-007 §4): replacing a `.dylib` *inside a signed app bundle* would invalidate that bundle's
signature — irrelevant here since PowerVoice never bundles LAME itself, only ever loads the
system's copy.

### LV2 plugin support

LV2 plugins need the **lilv** library at runtime. PowerVoice doesn't bundle it: the plugin sandbox
loads it the first time an LV2 plugin is scanned or loaded (`liblilv-0.so.0` / `liblilv-0.so` on
Linux; `liblilv-0.0.dylib` / `liblilv-0.dylib`, including the Homebrew folders, on macOS). If it's
missing, LV2 scans and loads fail with an in-app message and every other feature, including CLAP and
VST3 plugins, keeps working. **LV2 is not supported on Windows** (the backend is compiled out there).

**Linux (.deb)**: `liblilv-0-0` is a `Recommends` of the deb package, so `apt` installs it by default.
Manually: `sudo apt install liblilv-0-0` (Debian/Ubuntu), `sudo dnf install lilv-libs` (Fedora) or
`sudo pacman -S lilv` (Arch).

**Linux (AppImage)**: install the system lilv package as above; the AppImage doesn't bundle it.

**Linux (Arch)**: there's no PKGBUILD yet. When one is added it should declare
`optdepends=('lilv: LV2 plugin support')`.

**macOS**: `brew install lilv`.

**Overriding the library path**: set `POWERVOICE_LILV` to an absolute path to a custom lilv build
(advanced use only; normally unnecessary).

### JSFX support (vendored ysfx)

JSFX effects are hosted through **ysfx** (the JoepVanlier fork's Apache-2.0 library, with Cockos'
WDL/EEL2), vendored at a pinned commit in `third_party/ysfx/` (see its `PROVENANCE.txt`) and
compiled from source by `crates/ysfx-sys/build.rs` with the `cc` crate — into
`powervoice-sandbox` only. There is no runtime dependency to install, but **building needs a C and
C++17 compiler** (`gcc`/`g++` or `clang`/`clang++`; Debian/Ubuntu `build-essential`, Arch
`base-devel`, macOS Xcode command-line tools). It's built for unix on x86-64 and aarch64 (the EEL2
JIT back ends vendored). **JSFX is not supported on Windows**: `vox-ysfx-sys` compiles nothing
there (so `just check-cross` needs no C++ cross-compiler) and the sandbox answers "JSFX effects
aren't supported on this platform". A Windows build would need upstream's MSVC + NASM (or its
portable, non-JIT) EEL2 configuration — a follow-up.

## Cross-compiling a Windows check from Linux (`just check-cross`)

`just check-cross` runs `cargo check --target x86_64-pc-windows-gnu` over every crate that has no
Tauri/GTK dependency (`vox-module-api`, `vox-dsp`, `vox-modules`, `vox-rack`, `vox-engine`,
`vox-io`, `vox-project`, `vox-testkit`, `powervoice-cli`) — a fast way to catch
Windows-incompatible code (e.g. Unix-only syscalls) without a full MSVC/mingw toolchain. It needs
the Rust target installed:

```sh
rustup target add x86_64-pc-windows-gnu
```

`cargo check` type-checks and borrow-checks without linking, so this works even without a
`x86_64-w64-mingw32-gcc` cross-linker installed. It does **not** cover `src-tauri` (Tauri/WebView
code is platform-specific by nature and isn't meant to cross-check cleanly) — that's only ever
verified by an actual Windows build (see above).

## Notices and the shortcuts table

Two generated files are committed (not built fresh every time, so `git diff` shows exactly what
changed when a dependency or the shortcut registry changes):

- `THIRD_PARTY_NOTICES` (+ its copy at `ui/src/lib/help/thirdPartyNotices.generated.txt`, shown in
  Help → About) — regenerate with `just notices` after any dependency change.
- `docs/shortcuts.md` (T-701) — regenerate with `just shortcuts-table` after any change to
  `ui/src/lib/shortcuts/registry.ts` or a `shortcut.*` label in `ui/src/lib/i18n/en.json`.

`just check` fails if either is stale (`python3 scripts/notices/generate.py --check` and
`node scripts/docs/generate_shortcuts.mjs --check`, T-701).

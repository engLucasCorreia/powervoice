# PowerVoice

PowerVoice is a focused, cross-platform (Windows, macOS, Linux) desktop editor for **simple
voice-over work**: record mono voice, clean it up, shape it with a non-destructive effects rack,
hit a loudness target, and export — without the complexity of a full multitrack DAW. Think of it
as a lightweight alternative to Adobe Audition's Waveform Editor.

- Single-file waveform editor (one mono file open at a time)
- Record with punch-in / punch-and-roll, latency-calibrated
- Non-destructive effects rack: noise gate, spectral noise reduction, parametric EQ, dynamics
  (compressor/expander/limiter), true-peak limiter
- One-click peak/LUFS normalize, ACX compliance check
- Open/save WAV (16/24/32-bit float); import/export FLAC; import MP3/M4A(AAC)/OGG; MP3 export via
  an optional system LAME install
- Markers, undo/redo, autosave and crash recovery

See [`docs/user-guide.md`](docs/user-guide.md) for how to use it, and
[`docs/building.md`](docs/building.md) if you want to build it yourself on Windows or macOS.

## Install

### Linux

`just build` (see below) produces, under `target/release/bundle/`:

- **AppImage** (`appimage/PowerVoice_<version>_amd64.AppImage`) — download, `chmod +x`, run. No
  installation, no root needed. Bundles its own copy of WebKitGTK/GTK3 and their dependencies.
- **.deb** (`deb/PowerVoice_<version>_amd64.deb`) — `sudo apt install ./PowerVoice_<version>_amd64.deb`
  (or `sudo dpkg -i` + `sudo apt -f install` to pull in dependencies). Installs a desktop entry,
  icons, and file associations for `.wav`/`.flac`/`.mp3`/`.m4a`/`.ogg`.
- An **.rpm** is produced too as a side effect of Tauri's default bundle targets, though it isn't a
  ticket requirement and is untested on an actual RPM-based distro.

Runtime dependencies the .deb declares: `libasound2` (ALSA), plus WebKitGTK/GTK3 (auto-detected by
the bundler). MP3 export additionally needs `libmp3lame0` (declared as a `Recommends`, not a hard
dependency — see [Troubleshooting](docs/user-guide.md#troubleshooting) in the user guide).

### Windows / macOS

Not distributed as pre-built installers yet (no code-signing/notarization set up — PROMPT.md's v1
scope and this ticket's "Out" list both exclude that). Build from source: see
[`docs/building.md`](docs/building.md).

## Building from source

```sh
git clone <this repo>
cd audition
npm ci --prefix ui
just build       # release build + Linux bundles (AppImage, .deb)
```

`just dev` runs the app in dev mode. `just check` runs the full test suite (Rust + Svelte/TS).
See [`docs/building.md`](docs/building.md) for Windows/macOS prerequisites, and the
[Commands table](CLAUDE.md#commands) for every `just` recipe.

## License

PowerVoice is dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your
option. It's built on a number of open-source components — see
[`THIRD_PARTY_NOTICES`](THIRD_PARTY_NOTICES) (also shown in the app under Help → About → Third-party
notices) and [`docs/adr/ADR-007-licensing.md`](docs/adr/ADR-007-licensing.md) for the linking
policy behind it.

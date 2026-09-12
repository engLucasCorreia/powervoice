# ADR-007 — Dependency & plugin-format licensing
- Status: proposed
- Date: 2026-09-12
- Deciders: owner, orchestrator

## Context
PROMPT §3.7 requires recording the licenses of the VST3 SDK, LV2, CLAP, `ysfx`/WDL and LAME, and
stopping for owner sign-off before the VST2 adapter. PROMPT §6 escalates any dependency with licensing
implications to the owner. The project license has not been chosen yet (MEMORY). This ADR sets the
linking policy, lists every planned dependency, and turns the risky items into explicit owner
questions.

Licenses were checked on 2026-09-12, mostly from the crates.io API `license` field of the latest
version. URLs are cited inline. ⚠ marks items the implementing ticket must re-verify.

## Decision

### 1. Linking policy
| Class | Rule |
|---|---|
| Permissive (MIT, Apache-2.0, BSD, ISC, 0BSD, zlib) | Static linking allowed. Ship license texts and copyright notices, plus Apache `NOTICE` files, in `THIRD-PARTY-NOTICES` (packaged and shown in About). |
| MPL-2.0 | Allowed, unmodified, from crates.io. See §3. |
| LGPL (any version) | **Only as a separately replaceable shared library, loaded at runtime.** Never statically linked, never vendored into our binaries. |
| GPL (any version) | **Never linked into the editor binary.** Allowed only in a *separate program*: the user-installed Carla, or the `plugin-sandbox` binary if the owner accepts the process-boundary argument (§5). |
| System libraries (ALSA, PipeWire, JACK, WebKitGTK/GTK, WebView2, CoreAudio) | Dynamic, provided by the OS. If a package bundles them (AppImage), T-705 ships their license texts and source pointers. |

Compliance tooling (proposal, not added): `cargo-deny` license allowlist in `just check`,
`cargo-about` to generate `THIRD-PARTY-NOTICES`, and an npm license check for `ui/`. Their licenses
need checking when they are adopted.

### 2. Dependency table
| Component | Version | License | Where / linking | Obligations / notes |
|---|---|---|---|---|
| tauri / wry | 2.11.5 / 0.57.0 | Apache-2.0 OR MIT | `src-tauri`, static | notices |
| serde, serde_json, thiserror, anyhow | 1.x / 2.0.20 / 1.0.104 | MIT OR Apache-2.0 | static | notices |
| tracing, tracing-subscriber | 0.1.44 / 0.3.23 | MIT | static | notices |
| cpal | 0.18.2 | Apache-2.0 | `engine`, static | Apache text + NOTICE; mark any modification |
| alsa / jack / pipewire (cpal backends) | 0.12.1 / 0.13.5 / 0.10.1 | Apache-2.0/MIT, MIT, MIT | static bindings | notices; the C libraries are system libs (libasound LGPL-2.1+, libjack LGPL ⚠, libpipewire MIT ⚠), dynamic |
| windows / coreaudio-rs / libc / rustix | 0.62.2 / 0.14.2 / 0.2.189 / 1.1.4 | MIT OR Apache-2.0 (rustix adds an LLVM-exception option) | static | notices |
| rtrb | 0.4.0 | MIT OR Apache-2.0 | `engine`, static | notices |
| rayon (ADR-002 worker pool) / crc32fast (ADR-004 journal) | 1.12.0 / 1.5.1 | MIT OR Apache-2.0 | `engine` / `project`, static | notices |
| basedrop | 0.1.3 | MIT OR Apache-2.0 | **not used** (ADR-002 uses a return ring instead) | — |
| assert_no_alloc | 1.1.2 | BSD-1-Clause | tests and debug builds only | notice if shipped in debug builds |
| hound | 3.5.1 | Apache-2.0 | `io`, static | Apache text |
| **symphonia** | 0.6.1 | **MPL-2.0** | `io`, static | §3 |
| flacenc | 0.5.1 | Apache-2.0 | `io`, static | Apache text |
| rubato | 5.0.0 | MIT OR Apache-2.0 | `io`, static | notices |
| realfft / rustfft | 3.5.0 / 6.4.1 | MIT / MIT OR Apache-2.0 | `dsp`, static | notices |
| ebur128 | 0.1.10 | MIT | `testkit`/`dsp`, static | notices |
| memmap2 | 0.9.11 | MIT OR Apache-2.0 | `project`, static | notices |
| ts-rs / tauri-specta (ADR-003 picks one) | 12.0.1 / 1.0.2 | MIT | build-time codegen | notices |
| divan | 0.1.21 | MIT OR Apache-2.0 | benches only | none shipped |
| libloading | 0.9.0 | ISC | `io` (LAME loader) | notice. **New dependency; approved by this ADR** |
| **libmp3lame (LAME)** | 3.100 / 4.0 | **LGPL-2.0-or-later** | **runtime-loaded shared lib** | §4 |
| mp3lame-encoder / mp3lame-sys | 0.2.5 / 0.1.11 | LGPL-3.0 | **not used** | §4 |
| Svelte; Vite, Vitest, svelte-check | 5.57; dev | MIT ⚠ | UI bundle; dev-only tools | Svelte notice in the bundle's licenses file |
| **M8** CLAP headers | 1.2.10 | MIT | via clack | notice |
| clack-plugin / -host / -extensions | 0.2.0 | MIT OR Apache-2.0 | `plugin-host`, `plugin-sandbox`, `module-clap` | notices; pin exact versions (API not frozen) |
| **VST3 SDK** / `vst3` (coupler-rs) | 3.8.1 / 0.3.0 | MIT / MIT OR Apache-2.0 | `plugin-sandbox` | §6 |
| LV2 spec, lilv / livi | — / 0.7.5 | ISC / MIT | `plugin-sandbox` | notices; T-807 records how lilv is linked ⚠ |
| **ysfx** (JoepVanlier fork) + WDL/EEL2, dr_libs, stb, json | master | **Apache-2.0** (library); plugin build GPL-3.0 | `plugin-sandbox` only | §5 |
| Carla | — | GPL-2.0-or-later | **not linked**; separate user-installed program/plugin | §7 |
| interprocess / postcard (candidates, ADR-008) | 2.4.4 / 1.1.3 | 0BSD OR Apache-2.0 / MIT OR Apache-2.0 | `plugin-host`, `plugin-sandbox` | need orchestrator approval |
| zip / sha2 (ADR-006 install flow) | 8.6.0 / 0.11.0 | MIT / MIT OR Apache-2.0 | `plugin-host` | need orchestrator approval |
| iceoryx2 / shmem-ipc | 0.9.3 / 0.3.0 | MIT OR Apache-2.0 / Apache-2.0/MIT | reference only (ADR-008) | — |
| **Avoid:** `vst3-sys` (RustAudio), nih-plug's VST3 export | — | GPL-3.0 | never | would make our binaries GPL |

### 3. symphonia (MPL-2.0, file-level copyleft)
- We use it unmodified from crates.io, statically linked into `io`. MPL-2.0 copyleft applies per
  file.
- Our own files keep our license.
- When distributing executables we must tell recipients where the source of the MPL-covered files
  is (MPL §3.2). A pointer to the exact crate version's source satisfies this for unmodified use.
- If we ever patch a symphonia file, that file must be published under MPL-2.0. Prefer upstreaming.
- The MPL notice goes in `THIRD-PARTY-NOTICES`.
- Standard MPL-2.0 is compatible with both MIT and GPL-3.0 project licenses. T-202 verifies there is
  no "Incompatible With Secondary Licenses" notice. ⚠
- Separate from copyright: AAC decoding may carry **patent** considerations in some jurisdictions. We
  flag this before public distribution (open question 4). MP3 patents expired in 2017.

### 4. LAME: runtime-loaded libmp3lame, **not** `mp3lame-encoder`
**Findings:**
- `libmp3lame` is **LGPL-2.0-or-later**. `lame.h`: "either version 2 of the License, or (at your
  option) any later version" (https://raw.githubusercontent.com/rbrito/lame/master/include/lame.h).
  Arch packages `lame` 4.0-1 as LGPL-2.0-only, providing `libmp3lame.so.0`
  (https://archlinux.org/packages/extra/x86_64/lame/). LAME 4.0 was released on 2026-07-11
  (https://www.free-codecs.com/news/after-nine-years-frozen-lame-is-moving-again.htm).
- The `mp3lame-encoder` 0.2.5 and `mp3lame-sys` 0.1.11 crates are themselves **LGPL-3.0**.
  `mp3lame-sys`'s `build.rs` **always compiles the bundled LAME 3.100 and links it statically**:
  autotools with `cargo:rustc-link-lib=static=mp3lame` on Unix, and `cc` with `shared_flag(false)` on
  Windows. There is no system/dylib option
  (https://github.com/DoumanAsh/mp3lame-sys, https://crates.io/crates/mp3lame-encoder).
- Using those crates would therefore statically link LGPL code into our binary, twice over (C LAME and
  the LGPL-3.0 Rust wrapper). Rust has no practical "relink with a modified library" story for static
  archives. **They fail the "dynamic linking" decision.**

**Decision:**
- `io` contains a small hand-written FFI table for the public LAME API:
  - `lame_init`, `lame_set_in_samplerate`/`out_samplerate`, `lame_set_num_channels`,
    `lame_set_mode(MONO)`, `lame_set_brate`, `lame_set_VBR*`, `lame_set_quality`;
  - `lame_init_params`, `lame_encode_buffer_ieee_float`, `lame_encode_flush`,
    `lame_get_lametag_frame`, `lame_close`, `id3tag_*`.
- The table is resolved at runtime with **`libloading`** (ISC): `libmp3lame.so.0`, then
  `libmp3lame.dll` / `libmp3lame.0.dylib`.
- Declaring those public signatures is interface use that LGPL permits (LGPL-2.1 §5 on header
  material; LGPL-3.0 §3). ⚠ owner/counsel confirm.
- Benefits:
  - The user can replace the library at will: the core LGPL requirement.
  - Our code stays under our license.
  - No C toolchain is needed at build time.
  - The app starts without LAME and MP3 export is disabled with an explanatory message.
- **Distribution (T-705):**
  - Linux: a system dependency (deb `Recommends: libmp3lame0`; Arch `lame`). An AppImage may bundle
    an unmodified `libmp3lame.so.0` as a separate file, with the LGPL text and the exact source URL.
  - Windows/macOS: ship an unmodified upstream build as a separate DLL/dylib next to the executable,
    with the LGPL text and a source pointer.
  - macOS: replacing a dylib inside a signed bundle invalidates the signature. T-705 documents the
    replacement procedure (re-sign, or load from `~/Library/Application Support/<id>/lib`).
- Rejected: forking `mp3lame-sys` to add dylib linking (it still links LGPL-3.0 Rust code
  statically); `lame-sys` 0.1.2 (links a dylib, but unmaintained since 2017, and a missing library
  would crash at startup instead of disabling one feature).
- **Owner confirmation required before M6 / T-605.**

### 5. ysfx: sandbox binary only (and a correction)
- `docs/references.md` and MEMORY say "ysfx (GPLv3)". The JoepVanlier fork's `LICENSE` is
  **Apache-2.0** (https://github.com/JoepVanlier/ysfx, https://raw.githubusercontent.com/JoepVanlier/ysfx/master/LICENSE).
- Only the **distributed plugin binaries** are GPLv3. `plugin_license/README`: "this distribution of
  the plugin is licensed under GPLv3". That build uses JUCE and `clap-juce-extensions`.
- The host library (`sources/` plus WDL/EEL2, `dr_libs`, `stb`, `json` from `thirdparty/`) is
  permissive. T-808 re-verifies each `thirdparty/` license ⚠ and must build **only the library**,
  never the JUCE plugin target.
- **Decision (unchanged): ysfx is linked only into `plugin-sandbox`.** The main reason is now crash
  isolation: JSFX are user scripts JIT-compiled to native code. It also contains any license surprise.
- The *process-boundary argument* (GPL code in a separate process talking over IPC doesn't make the
  editor a derivative work) only matters if a GPL component enters the sandbox, e.g. a Carla-based
  VST2 route. **Have it reviewed before public distribution** in that case.
- JSFX scripts are user-supplied; we don't ship any.

### 6. VST3
- The Steinberg VST3 SDK is **MIT since 3.8.0** (released 2025-10-20). The latest is
  **3.8.1** (tag `v3.8.1_build_84`, 2026-08-11)
  (https://github.com/steinbergmedia/vst3sdk/tags,
  https://www.soundonsound.com/news/steinberg-adopt-mit-license-vst3).
- **Use SDK ≥ 3.8.0 headers only, pinned at 3.8.1.** We host through the `vst3` crate (coupler-rs,
  MIT OR Apache-2.0), which is generated from the SDK headers. T-806 records which SDK version its
  bindings come from and regenerates from 3.8.1 if they predate 3.8.0. ⚠
- Obligations: the Steinberg MIT notice.
- The MIT license grants no trademark rights. Describing the feature as "hosts VST3 plugins" is
  nominative use. Using the VST logo would need Steinberg's trademark terms.

### 7. VST2: gated
- There is no license to obtain: Steinberg stopped distributing the VST2 SDK in 2018 and has sent
  DMCA takedowns over VST2 SDK material (https://pouet.net/topic.php?which=11441,
  https://news.ycombinator.com/item?id=25977447).
- Clean-room headers (e.g. FST, VeSTige) exist, but their legal status is a grey area. The FST license
  is not verified ⚠.
- **Decision:** T-811 stays **gated on the owner's legal sign-off**. The preferred route ships **no
  VST2 code of ours**: the user installs Carla (GPL-2.0-or-later, https://github.com/falkTX/Carla) and
  we reach VST2 plugins through Carla, either its plugin build (e.g. an LV2 "Carla-Rack" in our LV2
  adapter) or its `carla-bridge-*` binaries. T-811 verifies the route ⚠.
- Carla is never linked, and we copy none of its code.

### 8. Project license: owner decision (implications)
| | MIT OR Apache-2.0 (Rust norm) | GPL-3.0-or-later |
|---|---|---|
| Compatible with the deps above | Yes, given §1: LGPL loaded at runtime, MPL unmodified, GPL kept out of our binaries | Yes: Apache-2.0, MPL-2.0 (secondary license), LGPL, MIT/ISC/BSD all flow into GPL-3.0 |
| GPL components (Carla, JUCE parts, `vst3-sys`) | Must stay in separate programs; process-boundary review needed | Could be linked directly; the process-boundary question disappears |
| Others' proprietary forks and closed builds | Allowed | Forbidden (the owner can still dual-license code they own outright) |
| Third-party modules/plugins | Unaffected | Unaffected while they run out of process via CLAP; in-process loading would raise derivative-work questions |
| Obligations | Notices file | Notices, plus source for every binary distribution |

**GPL-2.0-only is not viable**: it is incompatible with the Apache-2.0 dependencies (cpal, hound,
flacenc). Recommendation: **MIT OR Apache-2.0**, unless the owner wants copyleft. The choice is needed
before the first public distribution or outside contribution, and it doesn't block development.

## Consequences
**Positive**
- Every risky component is confined by construction: LGPL loaded at runtime, GPL out of our binaries,
  foreign code in the sandbox.
- The LAME approach also makes MP3 export optional instead of a startup requirement.
- Both candidate project licenses remain open.

**Negative**
- We maintain a small LAME FFI table and a runtime loader.
- Binary packages must carry LAME, notices and source pointers.
- VST2 support depends on a third-party program.

**Follow-ups**
- T-605: LAME loader.
- T-705: notices, packaging, macOS dylib replacement.
- T-202: MPL check.
- T-806: VST3 header version.
- T-808: ysfx library-only build and thirdparty licenses.
- T-811: gated.
- Correct "ysfx (GPLv3)" in `docs/references.md` and MEMORY.

## Alternatives considered
- **`mp3lame-encoder`:** static only and LGPL-3.0 itself. Rejected (§4).
- **Build-time dynamic linking (`-l dylib=mp3lame`):** compliant, but the app fails to start without
  the library. Rejected in favour of runtime loading.
- **Pure-Rust MP3 encoder:** none of comparable quality is known. Would need research; revisit if the
  owner rejects LAME.
- **Linking ysfx into the editor now that it is Apache-2.0:** rejected for crash-isolation reasons
  (ADR-008).

## Open questions (owner)
1. **LAME:** confirm runtime-loaded `libmp3lame` (LGPL-2.0-or-later), shipped as a separate replaceable
   library on Windows/macOS/AppImage and as a system dependency on deb/Arch. **Needed before M6.**
2. **Project license:** MIT OR Apache-2.0 (recommended) or GPL-3.0-or-later. Needed before any public
   distribution.
3. **VST2:** approve (or not) the Carla-based route in T-811.
4. Before public distribution: counsel review of the process-boundary argument (only if GPL code
   enters the sandbox) and of AAC-decoding patent exposure.

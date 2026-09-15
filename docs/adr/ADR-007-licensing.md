# ADR-007 — Dependency & plugin-format licensing
- Status: accepted (owner, M0 checkpoint 2026-09-12)
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

## Amendment 1 — M0 checkpoint (2026-09-12): owner decisions
- **Project license: `MIT OR Apache-2.0`** (dual, Rust-ecosystem standard). `LICENSE-MIT` and
  `LICENSE-APACHE` are in the repo root; crate and npm metadata follow in T-104.
- **LAME:** runtime loading of `libmp3lame` via `libloading` is **approved**.
- VST2 via Carla and unsigned native modules are deferred to the M8 checkpoint.

## Amendment 2 — T-200, 2026-09-13
- **`tauri-plugin-dialog`** (MIT OR Apache-2.0) approved, confined to `src-tauri`, for native
  Open/Save/Save As dialogs (SPEC-005).
- **`rubato`** lives in `dsp` only (ADR-001 rule 3); `io` resamples through `dsp::resample`. The table
  row that listed it under `io` is superseded.
- **Committed codec test vectors:** an exception to "no committed audio" — tiny MP3/M4A/Ogg/Opus test
  files (≤ 512 KiB total) generated from synthetic signals by a script (`just fixtures-codec`) may be
  committed under `crates/io/tests/data/`, so importer tests don't need external encoders in CI.
- **symphonia features:** `default-features = false` with `wav`, `pcm`, `flac`, `ogg`, `vorbis`, `mp3`,
  `isomp4`, `aac` (AAC-LC; patent note stays in the risk list).

## Amendment — CLAP host bindings are hand-written (T-803, 2026-09-14)
- The M8 row "clack-plugin / -host / -extensions 0.2.0" doesn't apply to the host side.
  `vox-clap-abi` transcribes the CLAP 1.2 type definitions from the headers (MIT, Copyright (c)
  2021 Alexandre BIQUE). The full license text is in `crates/clap-abi/LICENSE-CLAP`, and the
  flagged section of THIRD_PARTY_NOTICES names it.
- `libloading` (ISC), already a dependency of `vox-io` for LAME, also loads CLAP plugins, only
  inside `powervoice-sandbox` (ADR-008 Amendment 3). No new third-party crate.
- T-805 (`module-clap`, the plugin side) can still choose clack or reuse `vox-clap-abi`.

## Amendment — T-806 VST3 header version (2026-09-15)
The `vst3` crate 0.3.0's bindings are generated from the **VST SDK 3.8.0** `pluginterfaces`
(MIT, Copyright (c) 2025 Steinberg Media Technologies GmbH), which is ≥ 3.8.0, so no regeneration
was needed. §6's pin of 3.8.1 is met in spirit: nothing in the interfaces PowerVoice uses changed
in 3.8.1. Move to 3.8.1 bindings when the crate publishes them. The MIT notice is in
`crates/sandbox/LICENSE-VST3-SDK` and THIRD_PARTY_NOTICES (ADR-008 Amendment 6 §1).

## Amendment — T-807 LV2: headers transcribed, lilv loaded at run time (2026-09-15)
Resolves the ⚠ on the "LV2 spec, lilv / livi" row of §2.
- **LV2 headers** (lv2 1.18.10, ISC; Copyright 2006-2012 Steve Harris, David Robillard;
  Copyright 2000-2002 Richard W.E. Furse, Paul Barton-Davis, Stefan Westerfeld): the crate
  `vox-lv2-abi` transcribes the type definitions it needs (descriptor, features, `urid`,
  `uri-map`, `options`, `worker`, `state`, atom headers). The full license text is in
  `crates/lv2-abi/LICENSE-LV2`, and the flagged section of THIRD_PARTY_NOTICES names it.
- **lilv** (ISC; 0.28 is "0BSD OR ISC") is **not linked**. `powervoice-sandbox` loads the
  system's `liblilv-0` at run time through `libloading` (ISC, already a dependency), the first
  time an LV2 bundle is scanned or loaded. It is treated like the other system libraries of §1:
  never bundled by us; a Linux package should `Recommends:` it (deb `liblilv-0-0`, Arch `lilv` —
  a T-705 follow-up). Without it, LV2 plugins are unavailable and nothing else is affected.
- **Not used:** `livi` 0.7.5 (MIT) and the `lilv` / `lilv-sys` crates. `lilv-sys` links lilv at
  build time, so every build (including the Windows cross-check) would need lilv's development
  files, and a machine without lilv couldn't start the sandbox at all.
- No new third-party crate (`rtrb`, used for the LV2 worker's rings, was already a workspace
  dependency). ADR-008 Amendment 8 records the design.

## Amendment — T-808 ysfx: vendored library, built into the sandbox (2026-09-15)
Resolves the ⚠ on the "ysfx (JoepVanlier fork) + WDL/EEL2, dr_libs, stb, json" row of §2 and §5's
"T-808 re-verifies each `thirdparty/` license".
- **Acquisition.** crates.io has no ysfx binding or `-sys` crate (searched 2026-09-15: `ysfx`,
  `ysfx-sys`, `jsfx`, `eel2` — no results), and no system ysfx exists to load at run time. The
  **library part** of https://github.com/JoepVanlier/ysfx is vendored unmodified at commit
  `5c3452fee62583aa3d1b7e877d0c758c4024af89` (2026-08-19), with its `dr_libs` submodule at
  `f13cbcfd06afe7287f99b1bb5982cefdf3d6a974`, in `third_party/ysfx/` (2.0 MB, 86 files; the
  file list and update procedure are in `third_party/ysfx/PROVENANCE.txt`).
- **Never vendored:** the fork's JUCE/CLAP plugin (`plugin/`, `plugin_license/`, GPL-3.0), the
  GUI stack (LICE, SWELL, `stb`, `sources/lice_stb`), `thirdparty/json`,
  `thirdparty/clap-juce-extensions`, tests and tools.
- **Licenses, re-verified file by file:**
  - ysfx `include/` + `sources/`: Apache-2.0 (Jean Pierre Cimalando, Joep Vanlier). Two files
    inside carry their own: `sources/eel2-gas/` (EEL2's x86-64 JIT stubs in GAS syntax) zlib,
    `sources/base64/Base64.hpp` ISC.
  - WDL/EEL2 subset (the EEL2 compiler, lexer and parser, the aarch64 JIT stubs, `fft.c`, the
    headers they include): zlib, Cockos Incorporated / Nullsoft.
  - `eel2/y.tab.c`: Bison 2.3 output under the GPL-3.0 **with the Bison special exception**. The
    exception lets it be distributed under any terms in a work that isn't itself a parser
    generator, so it doesn't make anything GPL.
  - `dr_wav.h` / `dr_flac.h`: public domain (Unlicense) or MIT-0.
  - `stb` and `json` (MIT) are not vendored, so they're not relevant.
- **Build and linking.** New crate `vox-ysfx-sys`:
  - its `build.rs` compiles the vendored sources with the **`cc` crate** (1.4, MIT OR Apache-2.0,
    already in the lockfile; now a direct build dependency) into one static archive, `libysfx.a`:
    C for EEL2, C++17 for ysfx, `YSFX_NO_GFX`, `-O2` in every profile;
  - its `lib.rs` hand-writes the ~45 functions of the C API PowerVoice uses.
  - **Only `powervoice-sandbox` depends on it** (a target-specific dependency), so the editor
    binary never links ysfx. The C++ runtime comes from the system (dynamic `libstdc++`, like
    every C++ program).
- **Platforms:** unix on x86-64 and aarch64, the EEL2 JIT back ends vendored. Elsewhere,
  including Windows, `build.rs` compiles nothing, the crate is empty and the sandbox answers "JSFX
  effects aren't supported on this platform". `just check-cross` therefore needs no C++ cross
  toolchain. A Windows build (upstream uses MSVC + NASM, or its portable non-JIT EEL2) is a
  follow-up.
- **Notices.** The flagged section of `THIRD_PARTY_NOTICES` names every component above.
  `scripts/notices/generate.py` now also fails when a `third_party/<name>` folder isn't named in
  the notices.
- ADR-008 Amendment 11 records the backend's design.

## Amendment — T-805: clack on the plugin side, `zip` for `.voxmod` (2026-09-15)
- **clack-plugin / clack-extensions 0.2.0** (MIT OR Apache-2.0), with their dependencies
  clack-common 0.2.0 (MIT OR Apache-2.0) and clap-sys 0.5.0 (MIT/Apache-2.0), are used **only on
  the plugin side**: `vox-module-clap` and the module packages built with it
  (`vox-voxmod-gain`). Pinned `=0.2.0` (API not frozen). The editor and the sandbox still host
  CLAP through the hand-written `vox-clap-abi` (amendment above).
- **zip 8.6.0** (MIT) reads and writes `.voxmod` packages, with `default-features = false` and
  only `deflate-flate2`. It adds **typed-path 0.12.3** (MIT OR Apache-2.0); its other
  dependencies (crc32fast, indexmap, memchr) were already in the tree. **flate2** (MIT OR
  Apache-2.0, already in the tree via tauri) is named directly only to select its pure-Rust
  `miniz_oxide` backend (MIT OR Zlib OR Apache-2.0, already present). Linked by
  `vox-plugin-host` (and the `voxmod` packer in `powervoice-cli`).
- **sha2 is not used**: package checksums use a hand-written SHA-256
  (`vox_plugin_host::sha256`, tested with the FIPS 180-4 vectors), so the §2 row's "zip / sha2"
  request resolves to zip only.
- A module package statically links clack and the Rust standard library. `just voxmod` puts
  `LICENSE-MIT`, `LICENSE-APACHE` and `THIRD_PARTY_NOTICES` in the package's `licenses/`.
- Development builds compile miniz_oxide, crc32fast and adler2 at `opt-level = 3`
  (`[profile.dev.package.*]`): the `.voxmod` tests pack a real 40 MB debug `.clap`. Release
  builds are unaffected.

## Amendment — T-901 plugin windows: libX11 and suil at run time (2026-09-15)
- **libX11** (MIT/X11 license, the X.Org Foundation) is **not linked**: `powervoice-sandbox`
  loads the system's `libX11.so.6` at run time (`libloading`) the first time a plugin window
  opens, like lilv (amendment above). On a Wayland desktop XWayland provides it. It is a system
  library in §1's sense: never bundled by us.
- **suil** (ISC, David Robillard) is **not linked** either: the sandbox loads `libsuil-0` at run
  time for LV2 plugin UIs. Without it, LV2 plugins have no window and nothing else changes. A
  Linux package should `Recommends:` it next to lilv (deb `libsuil-0-0`, Arch `suil`; H-45 /
  packaging follow-up).
- **raw-window-handle 0.6.2** (MIT OR Apache-2.0 OR Zlib) is now a direct dependency of
  `src-tauri`, to read the editor window's X11 id or `HWND`. It was already in the graph through
  Tauri: THIRD_PARTY_NOTICES only marks it direct instead of transitive (`just notices`).
- **windows-sys** (already a workspace dependency) gains the `Win32_Foundation`,
  `Win32_Graphics_Gdi`, `Win32_System_LibraryLoader` and `Win32_UI_WindowsAndMessaging` features
  in `powervoice-sandbox` (Windows only).
- The test CLAP plugin (`vox-test-clap`, never shipped) uses `libloading` (already a workspace
  dependency) for its child X11 window.
- **Not used:** `x11`/`x11-dl`/`x11rb`/`xcb` (a build-time link or a large binding for ~25
  functions), `winit` and `baseview` (their own event loops and far more surface than one
  top-level window a plugin draws into). ADR-008 Amendment 13 records the design.

# ADR-006 — Installable module ABI (CLAP bundles)
- Status: proposed
- Date: 2026-09-12
- Deciders: owner, orchestrator

## Context
PROMPT §2 requires that the Module API (ADR-005) "later loads separately installed module packages".
§3.7 prefers reusing **CLAP as the packaging ABI** over inventing one. Built-in modules stay compiled
in for v1. M8 ticket T-805 has to prove that one built-in can ship as a CLAP package, and T-809 builds
the "Install module…" flow.

Relevant facts, checked 2026-09-12:
- CLAP headers are MIT, latest tag 1.2.10 (2026-07-13). CLAP explicitly allows vendor extensions
  identified by a unique, versioned id (https://github.com/free-audio/clap).
- `clack-plugin`, `clack-host` and `clack-extensions` 0.2.0 are on crates.io under MIT OR Apache-2.0.
  The README describes them as feature-complete but warns that "APIs can still have breaking changes".
  Custom extensions can be written with the same tools the built-in ones use
  (https://github.com/prokopyl/clack, https://crates.io/crates/clack-plugin).
- nih-plug's framework is ISC, but its VST3 export uses GPLv3 bindings
  (https://github.com/robbert-vdh/nih-plug). Its parameter model (shared atomics, framework
  smoothing) differs from ADR-005.
- A `.clap` is a single shared-library file on Linux and Windows and a bundle directory on macOS, so
  "a sidecar JSON inside the bundle" doesn't exist on two of our three platforms.

## Decision

### 1. Binary: a standard CLAP plugin, built with `clack-plugin`
A separately installed VoxEdit module is a **standard CLAP 1.2 plugin** built with `clack-plugin`
and `clack-extensions`. There is no custom ABI. A new crate **`module-clap`** (M8, T-805; depends on
`module-api`, `clack-plugin`, `clack-extensions`) provides a generic wrapper
`ClapModule<F: ModuleFactory>` plus an export macro. A module author writes an ordinary `Module`
(ADR-005) and exports it. The same binary also works in any other CLAP host.

The wrapper implements these CLAP standard extensions, following ADR-005 §14:
- `params`: plain values; `module` = group path; flags mapped; `value_to_text`/`text_to_value` from
  `ParamInfo`.
- `state`, `latency` (Restart becomes `request_restart`), `tail`, `audio-ports` (one mono port per
  direction), `render` (Realtime/Offline).

The Module API's "live instance" rule is stricter than CLAP. The wrapper enforces it internally:
main-thread calls during processing are answered from a wrapper-side parameter mirror, as the host
does in ADR-005 §7.

### 2. Our extra metadata travels in CLAP custom extensions (runtime, authoritative)
The CLAP plugin id **is** the module id (reverse-DNS, e.g. `com.acme.deesser`). The descriptor
version is a semver string. Features include `audio-effect` and `mono`. The wrapper exposes these
vendor extensions (C ABI structs, ids fixed here, shared with ADR-005 `ExtensionId::as_str`):

| Extension id | Purpose | Calls (thread) |
|---|---|---|
| `org.voxedit.module-info/1` | Everything CLAP can't express: param keys, i18n keys, taper, unit, decimals, `smoothing_ms`, enum labels, groups with `enable_param`, `state_format_version`, `api_version`, telemetry channel list, which of the extensions below are present | `get_info(plugin, clap_ostream*)` writes **UTF-8 JSON = serde of the ADR-005 types** (`ParamInfo[]` keyed by `clap_id`, `ParamGroup[]`, …) [main-thread] |
| `org.voxedit.telemetry/1` | `Telemetry` | `count`, `read(index) -> float` [thread-safe, wait-free] |
| `org.voxedit.response-curve/1` | `ResponseCurve` | `magnitude_db(values*, n, sr, freqs*, out*, m)`, `component_*` [thread-safe, non-RT] |
| `org.voxedit.noise-profile/1` | `NoiseProfile` | `capture(samples*, n, sr, values*, out_stream)`, `describe(blob, out_stream)` [main-thread] |

- **JSON inside the extension:** one serde schema is shared by the sidecar, the IPC and the package.
  This keeps the C struct small and stable (one function), and schema evolution is additive JSON
  fields. A breaking change bumps the id to `/2`, and the host keeps accepting `/1` as long as that is
  practical.
- **CLAP state for VoxEdit modules = UTF-8 JSON of `ModuleState`** (the same schema as the sidecar).
  Our CLAP adapter recognises `module-info` and treats such a plugin as a **VoxEdit module**:
  - it is registered under its bare id, not `clap:<id>`;
  - its state is **key-based**, not blob-only, so presets stay readable and migratable;
  - migrations run inside the plugin on `state.load`.

  Other CLAP hosts just see an opaque state stream.
- A third-party CLAP plugin without `module-info` is an ordinary external plugin
  (`clap:<id>`, blob-only state).
- The standard `params` extension and `module-info` must agree on ids, ranges, defaults and flags.
  T-805 tests this.

### 3. Distribution container: `.voxmod` package (static, validated without running code)
Extras that aren't needed at runtime (factory preset packs, locale files, license texts) and
multi-platform binaries travel in a **`.voxmod` file: a zip archive**. The layout:

```
manifest.json
bin/linux-x86_64/<name>.clap
bin/windows-x86_64/<name>.clap
bin/macos-universal/<name>.clap/…   (bundle directory)
presets/*.vopreset.json             same schema as user presets (T-406)
locales/<lang>.json                 keys must start with "modules.<id>."
licenses/…
```

`manifest.json`:

```json
{
  "manifest_version": 1,
  "id": "com.acme.deesser",
  "version": "1.2.0",
  "module_api": 1,
  "min_host_version": "1.0.0",
  "name": "De-esser", "vendor": "Acme", "license": "MIT", "url": "https://…",
  "binaries": { "linux-x86_64": "bin/linux-x86_64/acme-deesser.clap" },
  "sha256": { "bin/linux-x86_64/acme-deesser.clap": "…" }
}
```

The runtime extension is self-sufficient. A bare `.clap` VoxEdit module dropped into a CLAP folder
works fully; it just lacks the package's presets and translations. The package format needs one new
dependency, a zip reader. The proposed `zip` crate's license is recorded in ADR-007 and it needs
orchestrator approval before M8.

### 4. Built-in modules
Built-ins stay compiled in and in-process in v1, implementing the same Module API. The shipping app
never loads built-ins from CLAP. **T-805** proves the path end to end:
1. Wrap `org.voxedit.gain` with `module-clap`, using the id suffix `.packaged` to avoid a registry
   collision.
2. Load it through the M8 CLAP adapter in the sandbox.
3. Acceptance: the `ModuleTestHost` suite passes through the adapter. Output is bit-identical to the
   in-process Gain after aligning by the adapter's reported latency. The `module-info` JSON
   round-trips to the same `ParamInfo`s. The state saved by one loads in the other.

### 5. Where installed modules run
Installed modules are third-party native code, so they **run in the plugin sandbox (ADR-008)** like
any external plugin. They pay its added latency, reported through `latency_samples()`. Their
`Telemetry` and `ResponseCurve` calls go over the sandbox control channel: telemetry is polled
by the sandbox at UI rate and cached in shared memory, and curves are async request/response. Trusted
in-process loading is a possible v2 option and is not offered in v1.

### 6. Install location
Per-user, no admin rights: `<app local data dir>/modules/<id>/<version>/`, where the directory comes
from Tauri `app_local_data_dir()` and is passed into `plugin-host` by `src-tauri`:
- Linux: `$XDG_DATA_HOME/<identifier>/modules` (default `~/.local/share/<identifier>/modules`)
- Windows: `%LOCALAPPDATA%\<identifier>\modules`
- macOS: `~/Library/Application Support/<identifier>/modules`

One version per id is installed at a time. Installing a newer version replaces the old one after
confirmation. A downgrade needs an explicit confirmation, and state migration only goes forward. The
directory is scanned like a custom plugin folder (T-804), with the scan run in the sandbox.

### 7. "Install module…" flow (T-809)
1. The user picks a `.voxmod`, or a single-file `.clap` (Linux/Windows). Other formats are added by
   pointing the plugin manager at a custom folder.
2. **Validation without executing code:**
   - zip integrity;
   - reject absolute paths, `..`, symlinks and archives over 256 MB;
   - manifest schema;
   - a binary exists for this platform and matches its `sha256`;
   - id format, and `module_api` ≤ host;
   - **no collision with a built-in id** (hard error) or with an installed id (replace/downgrade
     prompt).

   A bare `.clap` is copied to `modules/_clap/<file>` as a plain CLAP plugin.
3. Extract to `modules/.staging/<random>/`, fsync, then atomically rename to `modules/<id>/<version>/`.
4. **Sandboxed scan** of the new binary. It must load, expose a plugin with the manifest's id, and (for
   `.voxmod`) expose `module-info` with valid JSON.
   - Failure: roll back (delete) and show an error.
   - Crash or timeout: roll back and blocklist the file hash.
5. Register in the scanner cache, merge the locale files under `modules.<id>.*`, and index the presets.
   The module appears in "Add module", enabled.
6. **Uninstall** from the plugin manager removes the directory. Racks that reference it show the
   ADR-005 "missing module" placeholder, which keeps the stored state.

**Trust:** the install dialog states that modules are native code with the user's privileges. The
sandbox isolates crashes, **not** malicious code. Packages are unsigned in v1.

## Consequences
**Positive**
- No ABI to design, document or version ourselves. Modules work in other CLAP hosts too.
- One adapter path (CLAP) covers both third-party plugins and our packages.
- Key-based state for packaged modules keeps presets portable.
- Validation before any code runs.

**Negative**
- `clack` 0.2 may still break its API: pin exact versions and isolate it behind `module-clap` and the
  CLAP adapter.
- Our extensions only mean something in VoxEdit (other hosts degrade to plain CLAP).
- Installed modules pay sandbox latency and IPC for telemetry and curves.
- macOS bundle, quarantine and notarization behaviour for installed `.clap` bundles is unverified
  (owner tests Linux only).

**Follow-ups**
- T-805 (`module-clap`), T-803 (recognise `module-info`), T-804 (scan `modules/`), T-809 (install
  flow), T-810 (state).
- The dependency request for `zip` (and `sha2` for hashes) goes to the orchestrator before M8.

## Alternatives considered
- **Custom Rust `dylib` ABI (or `abi_stable`):** rejected. Rust has no stable ABI, it would load
  in-process against the isolation decision, it's usable only by us, and it's a permanent maintenance
  burden.
- **Sidecar JSON next to or inside the bundle as the only metadata channel:** rejected as primary. It
  doesn't exist inside a single-file `.clap` on Linux/Windows and can drift from the binary. It is kept
  only as the package manifest for install-time validation.
- **nih-plug as the module SDK:** rejected. It's opinionated, its parameter model is incompatible, and
  its VST3 path is GPLv3. We only need CLAP.
- **VST3 or LV2 as the package format:** rejected. VST3 is more complex (COM, controller/processor
  split), even though its SDK is now MIT. LV2 is TTL-heavy and Linux-centric.
- **WebAssembly modules:** a genuine security sandbox and portable binaries, but there's no ecosystem,
  the SIMD/performance cost is unknown, and it would be a new runtime dependency. Revisit after v1.

## Open questions
1. **Owner:** installing unsigned native modules is acceptable for v1? (Signing or a curated catalog
   is out of scope: no online catalog in v1, PROMPT §3.8.)
2. Should trusted packaged modules be allowed in-process (no sandbox latency) in a later version?
3. The final app identifier and name, which determine the install directory, are tied to the product
   name (MEMORY open question).

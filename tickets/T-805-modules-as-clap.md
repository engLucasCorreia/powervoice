# T-805 — Our modules as CLAP packages (clack-plugin) + `.voxmod`

- **Tier:** Opus (blocking findings only)
- **Depends on:** T-803 (CLAP hosting), T-804/T-809/H-29 (catalog, install/uninstall), T-806–T-808 (format-neutral scanner).
- **Read first:**
  - CLAUDE.md, MEMORY.md (the T-803, T-804, T-809, H-29, T-806 and T-810 entries);
  - **docs/adr/ADR-006** (the whole thing, with its amendments):
    - §1 binary: a standard CLAP 1.2 plugin built with `clack-plugin` + `clack-extensions` (0.2.0, MIT OR Apache-2.0), with a new crate `module-clap` holding a generic wrapper over `module-api`;
    - §3 the `.voxmod` zip container, validated statically without running code;
    - the part where T-805 proves the path end to end, and the install flow;
  - ADR-005 (module API), ADR-007 (licensing: record clack and run `just notices`), ADR-008.

## Goal
One of PowerVoice's built-in modules (e.g. Gain, or the parametric EQ if it's straightforward) can be built as a standalone CLAP plugin and packaged as a `.voxmod`. PowerVoice installs it through Install Module… and runs it sandboxed, giving the same output as the built-in, sample for sample. The same `.clap` also loads in other CLAP hosts.

## Scope (in)
1. **`crates/module-clap`:** a generic `clack-plugin` wrapper that exposes any `module-api` `Module` as a CLAP audio-effect plugin:
   - params (with the module's units, text ↔ value), state (the module's blob), latency, tail;
   - mono ports (and stereo via the shim if ADR-006 says so);
   - sample-accurate param events;
   - PowerVoice's module-info extension, if ADR-006 defines one.
2. **An example package crate** (e.g. `crates/voxmod-gain`, a `cdylib`) built from the wrapper, plus a `just voxmod <crate>` recipe that builds the `.clap` and zips it into a `.voxmod` with the §3 layout (manifest, per-platform binaries, license, checksums).
3. **`.voxmod` install:** Install Module… accepts `.voxmod`:
   - validate the manifest, checksums, platform match and id/version statically, without running code;
   - extract the matching binary into the per-user install folder, then scan it in the sandbox;
   - handle collision and replace, and roll back and blocklist on failure;
   - uninstall removes the extracted files.
4. **Built-ins are never loaded from CLAP** (ADR-006). The package is an optional, separately installed module, so its id must not collide with the built-in's.

## Tests
- The wrapper exposes params/state/latency correctly, checked by loading the built package through our own CLAP host (T-803).
- It's bit-exact with the in-process built-in after the latency.
- A state round trip.
- `.voxmod` validation: bad manifest, checksum mismatch, wrong platform, path traversal in the zip. All are rejected with no code run.
- Install, scan, insert, then uninstall.
- An opt-in check that the `.clap` passes `clap-validator` if it's installed (skip otherwise).

`just check` and `just check-cross` must pass.

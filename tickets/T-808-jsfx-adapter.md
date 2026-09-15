# T-808 — JSFX adapter (ysfx, sandbox-only)

- **Tier:** Opus (FFI plus sandbox; blocking findings only)
- **Depends on:** T-803/T-806/T-807 (the backend pattern), T-804/T-806 (format-neutral scanner: `PluginFormat`, `scan_plugin_files_with`, per-format tiers, format-tagged cache and blocklist), H-29 (tiers/uninstall), H-36 (`service_with_events`), T-810 (state tests).
- **Read first:**
  - CLAUDE.md (RT rules, no system packages), MEMORY.md (the T-801–T-807, H-29, H-34, H-36 and T-810 entries: adding a format is compiler-guided through the `PluginFormat` matches; remember `INSTALL_EXTENSIONS`, the notices entry and the `check-cross` list);
  - docs/adr/ADR-007 (the ysfx/WDL licensing rows: record exactly how ysfx is built or linked, and run `just notices`), ADR-008 (+ amendments, the "JSFX sandbox-only" rule), ADR-005.

## Goal
JSFX effects (REAPER's text-based effect format, `.jsfx` or extensionless files with `desc:` headers) run sandboxed like the other formats. They are listed under "Plugins (JSFX)", automatable through their sliders, saved and restored, and a bad script never takes PowerVoice down.

## Scope (in)
1. **Library choice.** Candidates:
   - `ysfx` (C++ with the EEL2/WDL JIT), built from vendored sources by a `build.rs` with the `cc` crate;
   - loading a system ysfx at runtime (unlikely to be available).

   Prefer building ysfx into the sandbox binary only, never the editor, provided the licenses allow it (ADR-007). Check what the sandbox build needs (a C++ compiler is available here; no system packages). `just check-cross` must stay green: gate the JSFX backend off on Windows cross builds if the C++ toolchain there can't build it, and document that.
2. **Backend:**
   - load a script from its file plus the import search paths; compile it;
   - sliders become parameters (ranges, steps, enum sliders), with sample-accurate changes through block splitting;
   - `@init`/`@slider`/`@block`/`@sample` run through ysfx; `@gfx` is ignored (GUIs are T-901);
   - pin mapping to mono with the shim;
   - latency through `pdc_delay`, triggering a restart on change;
   - state = the slider values plus ysfx's serialized state (`@serialize`).
3. **Scan:** the JSFX paths are PowerVoice's own JSFX folder, `~/.config/REAPER/Effects` if present, and custom folders. Parse headers in the sandbox, one file per process. Effects only. Install and uninstall of `.jsfx` files, with the dependent imports copied if they're referenced relatively.
4. **Test scripts:** in-repo JSFX test scripts: a gain with one slider, a `pdc_delay` script, a stereo-only one, a `@serialize` one, and an infinite-loop or crash script for the watchdog/blocklist path.

## Tests
Mirror T-807:
- bit-exact against the in-process Gain after the latency;
- sample-accurate slider automation;
- a state round trip through the sidecar and a preset;
- a latency change triggers a restart;
- the scan finds the scripts in a temp path;
- a hanging or crashing script is blocklisted;
- install and uninstall;
- an opt-in real-script smoke test via `POWERVOICE_TEST_REAL_JSFX=<file>`.

`just check` and `just check-cross` must pass.

## Out
The JSFX `@gfx` GUI (T-901).

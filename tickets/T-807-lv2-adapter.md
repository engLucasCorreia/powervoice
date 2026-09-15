# T-807 — LV2 adapter

- **Tier:** Opus (FFI plus sandbox; blocking findings only)
- **Depends on:** T-803 (CLAP backend pattern), T-804 and T-806 (format-neutral scanner: `PluginFormat`, `scan_plugin_files_with`, per-format tiers, format-tagged cache/blocklist), H-29 (uninstall and duplicate tiers), T-809 (manager UI).
- **Read first:**
  - CLAUDE.md (RT rules), MEMORY.md (the T-801–T-806, H-29 and T-809 entries);
  - docs/adr/ADR-007 (licensing: LV2 headers and lilv are ISC; for any crate you use, record its license in an ADR-007 amendment and run `just notices`), ADR-008 (+ Amendments 1–6), ADR-005;
  - `crates/sandbox/src/{clap,vst3}/*`, `crates/plugin-host/src/*`.
- **Linux-first.** LV2 is mostly a Linux format. macOS and Windows builds must compile (`just check-cross`), and the LV2 backend may be compiled out there with a clear "not supported on this platform" path.

## Goal
Installed LV2 audio effects work like CLAP and VST3 ones: they are scanned, listed under "Plugins (LV2)", loaded sandboxed, automatable, and saved and restored, and a crash never takes PowerVoice down.

## Scope (in)
1. **Choose a library.** Candidates:
   - `livi` (Rust LV2 host built on lilv);
   - `lilv` through a small FFI wrapper;
   - hand-rolled Turtle/manifest parsing, with `dlopen` of `lv2_descriptor`.

   Prefer the smallest option that needs no system packages we can't assume. `lilv` may need `liblilv-0` at runtime: check what is installed here without sudo, and follow CLAUDE.md, which forbids installing system packages. If a library is needed at runtime, load it dynamically and degrade gracefully when it's missing. Explain the choice in an ADR-008 amendment.
2. **LV2 backend in the sandbox:**
   - instantiate; connect audio, control and atom ports;
   - the mono shim for stereo-only plugins;
   - control ports as parameters (ranges, enumerations, toggled, integer, logarithmic);
   - sample-accurate control changes (split the block at event offsets if the plugin has no event input);
   - latency through the `latency` port designation, triggering a restart on change;
   - state through the `state:interface` extension when the plugin offers it, otherwise the control values;
   - worker, URID map and options features as needed by common plugins, such as LSP, x42 and Calf.
3. **Scan:**
   - bundles in `$LV2_PATH`, `~/.lv2`, `/usr/lib/lv2`, `/usr/local/lib/lv2` and the macOS/Windows paths;
   - read `manifest.ttl` and plugin data sandboxed, one bundle per process;
   - audio effects only (no instruments);
   - the format-tagged cache and blocklist;
   - the "Plugins (LV2)" group in Add Module;
   - install/uninstall of `.lv2` bundles into `~/.lv2`.
4. **Test plugin:** a tiny in-repo LV2 test plugin, a `cdylib` plus `.ttl` files generated into a temp bundle by the tests: a gain with one control, latency, a stereo-only variant and a `crash-on-scan` copy. It mirrors `crates/test-clap`.

## Tests
Mirror T-803/T-806:
- bit-exact against the in-process Gain after the latency;
- sample-accurate control changes;
- a state round trip through the sidecar and a preset;
- a latency change triggers a restart;
- the scan finds the plugin in a temp LV2 path;
- a crashing bundle is blocklisted;
- install and uninstall;
- an opt-in real-plugin smoke test via `POWERVOICE_TEST_REAL_LV2=<bundle>`. If real LV2 plugins are installed here (e.g. `/usr/lib/lv2`), report which ones.

`just check` and `just check-cross` must pass.

## Out
LV2 GUIs (T-901), instruments/MIDI, JSFX (T-808).

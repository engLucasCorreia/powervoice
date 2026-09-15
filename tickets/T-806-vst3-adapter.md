# T-806 — VST3 adapter (vst3 crate) + moduleinfo scan

- **Tier:** Opus (FFI plus sandbox; blocking findings only)
- **Depends on:** T-803 (CLAP backend pattern, async load, `ParamText`), T-804 (`PluginCatalog`, scan/cache/blocklist), H-29 (tiers, duplicate-id policy, uninstall), T-809 (manager UI).
- **Read first:**
  - CLAUDE.md (RT rules), MEMORY.md (the T-801, T-802, T-803, T-804, H-29 and T-809 entries);
  - docs/adr/ADR-008 (§6: VST3 bundles with `moduleinfo.json` are indexed without loading code; Amendments 1–5), ADR-005 (+ amendments), ADR-006 (+ Amendment 1);
  - ADR-007 §6: VST3 SDK ≥ 3.8.0 (MIT), pinned at 3.8.1, hosted through the `vst3` crate (coupler-rs, MIT OR Apache-2.0, ~0.3). Never `vst3-sys` (GPL). "Hosts VST3 plugins" wording only, no trademark logo;
  - `crates/sandbox/src/clap/*`, `crates/plugin-host/src/{scan,catalog,install}.rs`.

## Goal
Real VST3 audio effects work exactly like CLAP ones: they are scanned, listed under "Plugins (VST3)", loaded sandboxed, automatable, and saved and restored, and a crash never takes PowerVoice down.

## Scope (in)
1. **VST3 backend** in the sandbox (`crates/sandbox/src/vst3/`):
   - load the bundle: the per-OS module entry (`GetPluginFactory`; `ModuleEntry`/`ModuleExit` on Linux, `bundleEntry` on macOS);
   - factory → the audio-effect class → `IComponent` + `IAudioProcessor` + `IEditController`, joining separate or combined controllers; connection points;
   - bus arrangements: mono, with the mono shim for stereo-only plugins as in CLAP;
   - `setupProcessing`, `setActive`, `process` with `ProcessData`, and parameter changes as sample-accurate `IParameterChanges` queues;
   - latency (`getLatencySamples`, with `restartComponent(kLatencyChanged)` → `RESTART_REQUEST`), tail;
   - state: component state plus controller state, wrapped with the plugin identity;
   - parameter info, and `getParamStringByValue` / `getParamValueByString` → `ParamText`;
   - host callbacks: `IComponentHandler` begin/perform/end edit, restart flags.
2. **Scan:**
   - `--scan <bundle> --format vst3` in the sandbox;
   - index bundles that carry `Contents/Resources/moduleinfo.json` without loading any code;
   - standard VST3 paths per OS (`~/.vst3`, `/usr/lib/vst3`, `/usr/local/lib/vst3`, and the macOS/Windows paths);
   - add VST3 functions in parallel to the CLAP ones, plus the H-29 tiers and duplicate policy;
   - cache, blocklist and health records carry the format.
3. **Registry/UI:** a "Plugins (VST3)" group in Add Module, the format badge in the manager (already supported), and Install Module accepting `.vst3` bundles into the per-user VST3 folder.
4. **Test plugin:** a tiny in-repo VST3 test plugin (`cdylib`, using the `vst3` crate's plugin-side bindings or hand-written ones): a gain with one parameter, state, 64 samples of latency, a stereo-only variant and a `crash-on-scan` copy. It mirrors `crates/test-clap` so the tests don't depend on installed plugins.

## Tests
Mirror T-803:
- bit-exact against the in-process Gain after the reported latency;
- sample-accurate automation;
- a state round trip through the sidecar and a preset;
- a latency change triggers a restart;
- the scan finds the plugin in a temp VST3 path, and a moduleinfo-only bundle is indexed without loading;
- a crashing bundle is blocklisted, not fatal;
- install and uninstall of a `.vst3` bundle;
- an opt-in real-plugin smoke test via `POWERVOICE_TEST_REAL_VST3=<path>`.

Run `just notices` after adding the crate. `just check` and `just check-cross` must pass (gate code per platform).

## Out
Plugin GUIs (T-901), LV2/JSFX (T-807/T-808), VST2 (gated).

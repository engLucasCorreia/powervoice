# T-803 — CLAP adapter (+ enumerate)

- **Tier:** Opus (FFI + sandbox; blocking findings only)
- **Depends on:** T-801 (`vox-sandbox-ipc`), T-802 (`powervoice-sandbox` with the `PluginBackend` trait, `ProxyModule`, watchdog, restart policy, slot status UI).
- **Read first:** CLAUDE.md, docs/adr/ADR-008 (+ Amendments 1–2), ADR-006 (installable module ABI — our own modules ship as CLAP bundles later, T-805), ADR-005 (+ Amendments 1–3, esp. `AdapterHealth`, state blobs, latency/restart), ADR-007 (licensing — the CLAP headers are MIT; name any crate you add and run `just notices`), MEMORY.md (T-801/T-802 notes: backend goes in `crates/sandbox/src/backend.rs`, instance creation is synchronous on the rack control thread → real plugins need async load with a loading status, plugin latency change → `HostRequest::Restart`, tests needing the sandbox binary live in `crates/sandbox/tests`, `pgrep -f` self-match gotcha; `/tmp` quota gotcha), crates/sandbox, crates/plugin-host, crates/rack.

## Goal
The owner can add a real CLAP audio-effect plugin (e.g. from Surge XT, Airwindows CLAP, LSP, or u-he) to the rack; it runs sandboxed, its parameters appear in the generic parameter UI, it saves/restores with presets and sidecars, and a crash never takes PowerVoice down.

## Scope (in)
- **CLAP backend** in the sandbox: load a `.clap` bundle (dlopen), `clap_entry` → factory → plugin; host callbacks (`clap_host`: request_restart / request_process / request_callback, thread check, log to the sandbox log); `clap.params` (list, get/set plain values, text ↔ value, flush), `clap.state` (save/load via the stream), `clap.latency`, `clap.tail`, `clap.audio-ports` (mono in/out; a stereo-only plugin gets our mono downmix/upmix shim), `clap.note-ports` ignored (effects only). Process with sample-accurate param events from the event ring.
- **Enumeration:** list plugins in the standard CLAP paths (Linux `~/.clap`, `/usr/lib/clap`, `$CLAP_PATH`; macOS/Windows paths documented), done **in a sandbox process** (a crashing bundle during scan must not crash the editor) → descriptors (`clap:<id>`, name, vendor, version, features) for the registry. A simple cache keyed by path + mtime; the full scanner/blocklist is T-804.
- **Async load:** instance creation runs off the rack control thread with a "Loading…" slot status; the slot becomes Running when activated (or Failed with the plugin's reason).
- **Rack/UI:** Add Module shows installed CLAP effects under "Plugins (CLAP)" (real ones, not behind the dev flag); parameters through the generic parameter UI; latency reported and compensated.

## Tests
- A tiny test CLAP plugin built in the repo (a `cdylib` implementing `clap_entry` for a gain effect with one parameter, state and latency) so tests don't depend on installed plugins: bit-exact vs in-process Gain after the reported latency; parameter automation sample-accurate; state round trip (sidecar + preset); latency change → restart; enumeration finds it in a temp CLAP path; a crashing bundle during scan is reported, not fatal.
- If a real CLAP plugin is installed on this machine, an opt-in smoke test (`POWERVOICE_TEST_REAL_CLAP=<path>`) that loads it and processes a block.

## Out
Plugin GUIs (T-901), VST3/LV2/JSFX (T-806–T-808), the scanner/blocklist UI (T-804, T-809).

`just check` and `just check-cross` must pass.

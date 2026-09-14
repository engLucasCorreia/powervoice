# T-802 — Sandbox process, watchdog, proxy module

- **Tier:** Opus (cross-process + RT; blocking findings only)
- **Depends on:** T-801 (`vox-sandbox-ipc`: shm layout, rings, wakeup, bounded host wait, crossfaded bypass, crash/hang monitor, test plugins), the rack (S3-01, T-103, H-15 placeholder slots), T-306/T-406 (module state in sidecars/presets).
- **Read first:** CLAUDE.md (RT rules), docs/adr/ADR-008-plugin-sandbox-outline.md (+ Amendment 1 — the implemented transport), ADR-006 (installable module ABI), ADR-005 (+ Amendments: Module trait, state blobs, latency, bypass, return ring), ADR-002 (+ Amendments 1–2), MEMORY.md (T-801 notes; RT notes; process-spawning tests live in their own test binary; `fs_util::pid_is_alive`; `/tmp` quota gotcha — clean shm/temp objects; T-705: any new dependency needs `just notices`; pre-commit `cargo fmt --all --check`), crates/sandbox-ipc, crates/module-api, crates/rack, crates/engine/src/rack_api.rs.

## Goal
An out-of-process plugin behaves like any other rack module to the host — parameters, state, latency, bypass — and a crashing or hanging plugin never takes the app or the audio down.

## Scope (in)
- **Sandbox host binary** (`powervoice-sandbox`): spawned per plugin instance (or per plugin bundle, per ADR-008), attaches the T-801 segment, loads a plugin through a small in-process "plugin backend" trait (T-803+ add CLAP/VST3/LV2/JSFX backends; here a built-in test backend wrapping T-801's test plugins), runs the processing loop, answers control RPCs (activate/deactivate, params list/get/set, state save/load, latency) over a control channel (a simple framed pipe/socket — no new heavy dependency).
- **Watchdog** (control thread side): heartbeat supervision, crash/hang detection from T-801's monitor, restart policy (restart once with the last good state; after repeated failures mark the slot "Plugin failed" and keep it bypassed), clean teardown on slot removal/app exit (no orphan processes, no leaked shm — verified).
- **Proxy `Module`** in the host: implements the Module API for a sandboxed plugin — `ParamInfo` mirrored from the plugin, parameter events forwarded through the event ring, `process()` = T-801 `HostEnd` (no alloc, bounded wait, crossfaded bypass on deadline miss), latency reported (plugin latency + one block of transport latency, per ADR-008), state blob = the plugin's state wrapped with the plugin id/format/version.
- **Rack integration:** a sandboxed slot can be added via a registry entry for the test backend (hidden behind a debug/dev flag in the Add Module menu until T-803+ ship real formats), appears in sidecars/presets like any module, and a missing/failed plugin loads as H-15's placeholder slot.
- **UI:** slot status (Running / Restarting / Failed / Not installed) with the failure reason and a Retry action.

## Tests
Gain backend through the proxy: bit-exact vs in-process Gain after the reported latency; kill -9 the sandbox mid-playback → host keeps playing (bypass), watchdog restarts with state, second failure → Failed; hang → detected within the heartbeat budget; slot removal and app shutdown leave no processes or shm objects; state round trip through sidecar and preset; no allocation on the audio thread with a sandboxed slot active. Process-spawning tests in their own test binary.

## Out
Real plugin formats and scanning (T-803/T-804), plugin GUIs (T-901).

`just check` must pass.

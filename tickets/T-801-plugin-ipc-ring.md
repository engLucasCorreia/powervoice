# T-801 — Plugin sandbox transport: shared-memory audio ring + wakeup shim + test plugins

- **Tier:** Opus (RT + cross-process; blocking findings only)
- **Depends on:** the rack (S3-01, T-103), ADR-005 (Module API), ADR-006 (installable module ABI), ADR-008 (plugin sandbox outline). First ticket of M8 (external plugins); T-802 (sandbox process, watchdog, proxy module) builds on it.
- **Read first:** CLAUDE.md (RT rules: the audio thread never blocks, allocates, locks or makes unbounded syscalls — the wakeup must be bounded and the host must fall back to bypass on a missed deadline), docs/adr/ADR-008-plugin-sandbox-outline.md and ADR-006 (the transport, message, and failure model they describe), ADR-002 (+ Amendment 1) and ADR-005 (+ Amendments), MEMORY.md (RT notes, return ring, `vox_module_api::test_util::no_alloc`; H-05/T-301/H-21 process-spawning test pattern — such tests live in their own test binary; `/tmp` quota gotcha), crates/module-api, crates/rack.

## Goal
A measured, robust building block for running audio plugins out of process: the host's audio thread hands a block to a sandbox process and gets the processed block back within the deadline — or bypasses cleanly — without ever blocking or allocating.

## Scope (in)
- New crate (e.g. `vox-sandbox-ipc`): a shared-memory region per plugin instance with SPSC audio block rings (host → plugin input, plugin → host output), a control/param event ring, and a heartbeat/state word; layout versioned and documented (ADR-008 amendment if the outline is refined).
- **Wakeup shim** per platform: Linux futex (or eventfd), macOS/Windows equivalents documented and stubbed behind a trait with a portable spin-then-yield fallback; host side waits at most the deadline budget (a configurable fraction of the block period), never unbounded.
- **Failure model:** missed deadline → the host outputs the dry block (crossfaded) and counts it; plugin crash (peer gone) and hang (heartbeat stale) are detected without blocking and reported to the control thread.
- **Test plugins** (tiny test binaries in the crate): `gain` (applies −6 dB), `crash` (aborts after N blocks), `hang` (stops responding), `slow` (misses deadlines randomly).
- **Benchmarks:** round-trip latency distribution (p50/p99/max) for 64/128/256/512-frame blocks at 48 kHz on this machine; report numbers.

## Tests
Gain plugin bit-exact round trip; crash/hang/slow → bypass with a crossfade, no allocation on the host audio thread (`no_alloc`), control thread gets the fault; shared-memory layout golden test; the process-spawning tests in their own test binary.

## Out
The sandbox process lifecycle/watchdog/proxy `Module` (T-802), real plugin formats (T-803+).

`just check` must pass.

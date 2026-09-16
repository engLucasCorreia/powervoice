# H-53 — Calibration success-path test

- **Tier:** Sonnet (no review loop)
- **Found by:** H-50. `src-tauri/src/calibration.rs`'s success path is correct by inspection (its `Result` precedes the terminal `Done`), but nothing tests it: the fake-hardware loopback rig lives in a private test module, `crates/engine/tests/punch.rs`'s `Rig`/`Loopback`.
- **Read first:** CLAUDE.md, MEMORY.md (the T-304, H-50, T-102 FakeBackend and T-401 `frame_time_ns` entries), `crates/engine/tests/punch.rs`, `crates/engine/src/backend/fake.rs`, `src-tauri/src/calibration.rs`, specs/SPEC-022 §2.14.

## Scope (in)
1. Move the loopback rig into shared test support so both crates can use it: a `test-util` module behind a feature in `vox-engine` (the pattern `vox_module_api::test_util` already uses), not a copy.
2. `src-tauri` test: a successful calibration reports its `Result` before the terminal `Done`, with a plausible measured offset, and the measured offset reaches Settings as the spec says.
3. Keep the existing failure-path test, and don't change production behaviour.

`just check` must pass.

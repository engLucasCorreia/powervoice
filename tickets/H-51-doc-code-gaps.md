# H-51 — Close the doc/code gaps found by T-706

- **Tier:** Sonnet (blocking findings only)
- **Read first:** CLAUDE.md, MEMORY.md (the T-706 entry), `docs/architecture/*` (they document today's code), the ADRs and specs named below.
- **Rule:** the code is the truth for what ships. Fix the documents, except where the code itself is wrong (items 3 and 6), which are real defects.

## Scope (in)
1. **ADR-001 §2 crate graph** is stale: no `engine→io`; `rack→modules` is dev-only; `plugin-host→rack` and `→sandbox-ipc` exist; the app also depends on dsp, io, modules, presets and plugin-host. Amend the ADR to match `just docs`' generated graph.
2. **ADR-001 §4:** export, ACX and processed-analysis jobs live in `src-tauri` services, not `vox-engine`; only bake and the shared range render are in the engine. Amend.
3. **Sessions path (ADR-004 §1 / ADR-001 §6):** sessions use `ProjectDirs::data_dir()`, which on Windows is the **roaming** profile, while modules use `app_local_data_dir`. Recordings must not sit in a roaming profile.
   - Decide and implement: sessions belong in the local data dir on Windows.
   - Migrate an existing session directory if one is found at the old path (on Windows only), or document why migration isn't needed.
   - Amend the ADR and note it in `docs/architecture/data.md`.
4. **ADR-002 §1:** there is no `rayon` pool (spectro threads plus per-job threads); telemetry is 60 Hz, not 30; the monitor ring is 65 536, not 8192 (Amendment 1). Amend.
5. **SPEC-013 / SPEC-016:** AutoGate, Expander and look-ahead are hidden and inert (`NOT_YET_AVAILABLE`), latency 0, telemetry channels read 0; the `TransferCurve` extension doesn't exist. Mark them deferred in the specs, with the state of each.
6. **Output meter vs analyzer tap** (`crates/engine/src/output.rs`): the meter excludes the dry monitor signal while the analyzer tap includes it, yet the comment says they're identical. Decide which is right (the meter should show what the user hears), fix the code or the comment, and add a test.
7. **Stale comments** in `src-tauri/src/loudness.rs` and `ipc/document_commands.rs` referencing a non-existent `edit_normalize_lufs`. Remove or correct.
8. **ADR-008:** §5's "no automatic restart" and §8's crate choices are superseded by amendments; the protocol is v3. Fold the amendments into the body or mark the superseded lines.

## Tests
- A test for item 3's path choice (per-OS, with a fake home).
- A test for item 6's meter/analyzer agreement (or documented difference).
- `scripts/docs/check.py` stays green; run `just docs` if generated sections change.

`just check` must pass.

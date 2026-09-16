# H-77 — Custom Dynamics panel and the binary curve frame (T-410 remainder)

- **Tier:** Opus (DSP-adjacent UI plus an IPC frame; blocking findings only)
- **From:** H-63's hand-off. The transfer graph, the extension and the draggable handles exist; what remains is T-410's own scope.
- **Read first:** CLAUDE.md, MEMORY.md (H-63's recipe and its hand-off list; H-03's gain-reduction meters and `VXMT`; H-58's live sections; H-41's meter ballistics — reuse `PeakBallistics`; H-43's frame scheduler; H-26 kit and T-708 tokens; H-59's VXPK golden-fixture pattern for a new binary frame; T-702 i18n lint; T-706's `just docs`), specs/SPEC-016 (§2.6, §4.12, AC-20, AC-22, AC-23), specs/SPEC-013, `ui/src/lib/transfer/*`, `ui/src/lib/rack/RackSlot.svelte`, `crates/engine/src/rack_api.rs`, `src-tauri/src/ipc/rack_*.rs`.

## Scope (in)
1. **`VXTC` binary frame** for the transfer curve as SPEC-016 §4.12 defines it, replacing the interim JSON on this path, with a Rust→TS golden fixture (the VXPK pattern) and the AC-23 timing test (512 points within the spec's budget).
2. **Custom Dynamics panel** (SPEC-016 §2.6, AC-20/AC-22): the global row with the latency readout, per-section panels in the spec's order, gain-reduction meters per section fed by the existing telemetry, and the AutoGate lamp.
3. **Operating-point dot** on the graph: input level plus total gain reduction plus effective makeup, as the spec describes. It needs panel-local data, so it belongs here rather than in the generic graph.
4. Always-on handle labels where they fit, and per-component curve overlays (the data already arrives in `components_db`).

## Tests
- The `VXTC` frame round trip and its golden fixture; AC-23's timing.
- AC-20 and AC-22.
- The meters and lamp against known telemetry.
- Panel layout at the usual widths, in Dark and Light, with no overlap (screenshots into the session scratchpad `h77/`).

`just check` must pass.

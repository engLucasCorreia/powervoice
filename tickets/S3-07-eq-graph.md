# S3-07 — EQ graph panel (Slice 3)

- **Tier:** Sonnet
- **Depends on:** S3-01 (rack panel + generic parameter UI), S3-03 (EQ module + `ResponseCurve`)
- **Spec:** SPEC-015 graph part, lean subset (MEMORY A-008).

## Scope (in)
Custom panel for the EQ slot: log-frequency axis 20 Hz–20 kHz, ±12 dB (±24 option); curve drawn only from the module's `ResponseCurve` via a `rack_response_curve(slot, points)` command (≤ 512 points; `VXRC` framing is hardening); draggable nodes per band (freq/gain) sending plain values via a `param_set_plain(slot, id, value)` command; mouse wheel = Q; double-click a node toggles the band; generic parameter panel stays available below. Vitest for node ↔ value mapping and command flows (mockIPC).

## Out
Live analyzer overlay (needs the analyzer T-208 + VXSA reconciliation), keyboard node navigation, expanded view.

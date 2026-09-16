# H-63 — `TransferCurve` module extension

- **Tier:** Opus (module API plus DSP and UI; blocking findings only)
- **From:** H-58 and H-51. SPEC-016 §4.11 and SPEC-013 describe a transfer-curve extension so a dynamics-style module can hand the UI its input→output curve; it doesn't exist in `module-api`'s `ExtensionId`/`Extension` enums, which blocks SPEC-016 AC-17, the gate UI and T-410's custom Dynamics panel with its gain-reduction display.
- **Read first:** CLAUDE.md (RT rules — the curve must be produced off the audio thread or from a lock-free mirror, never by blocking `process()`), MEMORY.md (T-005/ADR-005 extension pattern; H-03's gain-reduction meters and `VXMT`; H-58's live Expander/AutoGate and its `restart_requested_for` pattern; T-409's EQ `ResponseCurve` — the closest existing extension, follow it; H-26 kit, T-708 tokens, H-43 frame scheduler for any drawing), docs/adr/ADR-005 (extensions), specs/SPEC-016 §4.11 + AC-17, specs/SPEC-013, `crates/module-api/src/lib.rs` (ExtensionId/Extension), `crates/modules/src/{dynamics,noise_gate}.rs`, the EQ response-curve path end to end (module → IPC → `EqGraph`).

## Scope (in)
1. Define the extension in `module-api` following the EQ `ResponseCurve` precedent: a request for N points over a dB input range, returning output dB (plus whatever SPEC-016 §4.11 names — knee, threshold markers, per-section curves if specced).
2. Implement it for **Dynamics** (compressor, expander, gate sections as the spec defines) and the **Noise Gate**.
3. Plumb it to the UI the way the EQ curve is plumbed, and draw it: a transfer graph in the module's panel showing the curve, the current operating point if the spec asks, with theme tokens and on-demand drawing (H-43).
4. Cover SPEC-016 AC-17.

## Out
T-410's full custom Dynamics panel layout beyond the graph itself, and `VXMT` telemetry changes — say in your report what remains for it.

## Tests
- The curve maths against the module's own processing at several parameter sets (the curve must match what the module actually does, within a stated tolerance).
- Extension round trip through IPC.
- AC-17.
- Drawing tests at the level the EQ graph has.

`just check` must pass.

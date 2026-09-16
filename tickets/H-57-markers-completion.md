# H-57 — Markers: kinds, drag and region→selection (T-303 remainder)

- **Tier:** Sonnet (no review loop; blocking findings only)
- **From:** T-303's row — "S2-03 (done); rest: marker kind in the core model, drag, region→selection".
- **Read first:** CLAUDE.md, MEMORY.md (T-301 document model, S2-03 markers, T-206 selection + zero-crossing snap, H-27 follow, H-26 Menu/Dialog, H-18 fixtures, T-702 i18n lint, T-706 docs — run `just docs` if commands change), specs/SPEC-009 (marker kinds, drag, region markers, navigation ACs), specs/SPEC-004 (undo), `crates/project` (markers), the Markers panel and the ruler.

## Scope (in)
1. **Marker kinds in the core model** (cue, region, whatever SPEC-009 lists): stored in the document, written to and read from WAV cue/adtl chunks where the format allows, shown in the panel, and preserved through save, reopen and recovery.
2. **Drag** a marker (and a region's edges) on the ruler or waveform: snapping per the spec (including the zero-crossing setting where it applies), one undo entry per drag, and no drift at high zoom.
3. **Region → selection**: selecting a region marker sets the time selection (and the reverse, if the spec says so).
4. Menu items, shortcuts and i18n as the spec names them.

## Tests
- Kind round trip through save, reopen and the WAV chunks.
- Drag maths (pure) plus an interaction test, including snapping and undo.
- Region → selection.
- Applicable SPEC-009 acceptance criteria.

`just check` must pass.

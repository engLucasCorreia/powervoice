# H-64 — Marker follow-ups

- **Tier:** Sonnet (no review loop)
- **From:** H-57's deferrals.
- **Read first:** CLAUDE.md, MEMORY.md (H-57's marker model, drag geometry and the `activateMarker`/`jumpToMarker` distinction; H-43's frame scheduler; H-26's Menu and `axisLabels`; T-701's registry; T-206's selection; T-702's i18n lint), specs/SPEC-009 (§2.5 `drag_autoscroll_rate`, §2.13 case 3, and the panel sections), specs/SPEC-018, `ui/src/lib/waveform/markerDrag.ts`, the Markers panel, `crates/project` markers and the open path.

## Scope (in)
1. **Auto-scroll while dragging** a marker or region edge past the canvas edge, at SPEC-009 §2.5's rate, through the frame scheduler (animate only while dragging).
2. **SPEC-009 §2.13 case 3**: when a WAV's cue set differs from the matching sidecar, use the file's markers and show the spec's notice. The spec table credits T-306, but nothing implements it — confirm that first, then implement.
3. **Markers panel deferrals** from S2-03, as far as the spec defines them: filter, sort, Delete All / Delete Filtered, the rename shortcut, and virtualization if SPEC-009 AC-15's 10 000-marker case needs it (H-59 noted that AC can't be measured until the list virtualizes).

## Tests
- Auto-scroll maths and that it stops at the document edges.
- The §2.13 case-3 precedence with a fixture whose cues differ from its sidecar.
- Panel filter/sort/delete behaviour, and the 10 000-marker case if you virtualize.

`just check` must pass.

# H-72 — Amplitude ruler percent mode, and notices for unreadable WAV cues

- **Tier:** Haiku (two small, well-scoped gaps)
- **From:** H-60's audit.
- **Read first:** CLAUDE.md, MEMORY.md (H-35's vertical zoom and the amplitude ruler, H-26's `axisLabels` fitting and `units.ts` formatting with the true minus, T-708 tokens, H-60's note that malformed cues are dropped safely but silently, T-702's i18n lint, H-18 fixtures), specs/SPEC-006 §2.4 (the amplitude ruler and its percent mode), specs/SPEC-005 §2.5 (the marker/cue notices and their exact keys), `ui/src/lib/waveform/amplitudeAxis.ts`, `crates/io/src/wav.rs`, `src-tauri/src/document.rs` (open path).

## Scope (in)
1. **Amplitude ruler percent mode**: the toggle SPEC-006 §2.4 describes, switching the ruler between dBFS and percent, with collision-free labels in both and the choice persisted where the spec says (Settings or the sidecar view state — follow the spec, and update `ui/src/lib/test/fixtures.ts` for any new field).
2. **Cue notices on open**: emit `notice.open.markers_unreadable` and `notice.open.markers_out_of_range` (exact keys and wording from SPEC-005 §2.5) when a WAV's cue chunk is malformed or points outside the audio. The current behaviour — dropping them — is correct and must not change; it should just say so.

## Tests
- Ruler labels in both modes at several heights and zooms; the persisted choice round-trips.
- A WAV fixture with a malformed cue and one with an out-of-range cue each produce their notice, and the markers are still dropped safely.

`just check` must pass.

# H-78 — Refresh the documentation for everything since T-706/T-707

- **Tier:** Sonnet (writing plus verification)
- **Why:** the docs were written at T-706/T-707 and a lot has shipped since. They must not describe an older app.
- **Read first:** CLAUDE.md, MEMORY.md's "Ticket learnings" from H-41 onward (each entry says what changed), `docs/README.md`, `docs/user-guide.md`, `docs/how-it-works.md`, `docs/what-is-powervoice.md`, `docs/faq.md`, `docs/glossary.md`, `docs/architecture/*`, `docs/building.md`, `README.md`, and `scripts/docs/check.py` (links, anchors, Mermaid and generated sections are enforced).

## Scope (in)
Go feature by feature and update every document that mentions it, adding what's missing:
- **Meters** (H-41/H-48): the vertical output meter, peak/RMS behaviour, the clip latch.
- **Analyzer** (H-42): peak labels, voice diagnostics, Average/Compare, the Spectrum Inspector.
- **Themes/tour/plugins** — check they still match after H-69's renames and the plugin-window work (T-901, H-49).
- **Editing**: Insert Silence and the cross-document clipboard (H-56), the waveform right-click menu (H-66), markers with kinds, dragging and region→selection (H-57), zoom commands (H-35), loop playback (H-37), time formats and zero-crossing snap (T-206), the amplitude ruler's percent mode (H-72).
- **Effects**: Dynamics part 2 now live (H-58), the transfer-curve graph (H-63).
- **Files**: saving with progress and cancel, the free-space and size pre-flights, clearer save errors (H-70, H-60), progressive waveform during import (H-71), marker notices (H-72).
- **Platforms**: Windows and macOS installers now build on CI but are unverified (H-61/H-69); LV2 needs lilv; plugin windows need X11/XWayland (H-49).
- **Architecture docs**: the frame scheduler replacing the old draw loops (H-43), idle telemetry gating, the sandbox reset-chunk guarantee (H-55), `service_with_events` (H-36), the `TransferCurve` extension (H-63), `import_peaks_get` (H-71), `JobKind::Save` (H-70), and the case-collision guard (H-69). Run `just docs` so the generated crate graph and IPC tables are current.
- **Glossary/FAQ**: terms introduced since (transfer curve, dropout marker, sandbox, LTAS…), and any FAQ answer that is now wrong.

Every UI name must match `ui/src/lib/i18n/en.json` exactly — check, don't assume. Where a feature is platform-limited or unverified, say so plainly.

## Tests
`scripts/docs/check.py` passes; `just check` must pass. In your report, list every file you touched and the features you verified against the code rather than the ticket text.

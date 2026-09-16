# H-76 — Zoom, scroll and selection during an import

- **Tier:** Haiku (contained; verify against the code before reporting)
- **From:** H-71. SPEC-005 §2.3 item 4 says the view stays interactive while an import runs. Today the importing waveform draws progressively, but the view's own interactions — and the previous document's overlays — were left alone.
- **Read first:** CLAUDE.md, MEMORY.md (H-71's `import_peaks_get` and progressive drawing; H-43's frame scheduler; T-206's selection model; H-35's zoom commands), specs/SPEC-005 §2.3, `ui/src/lib/waveform/WaveformView.svelte`, the import job path and the import-peaks store.

## Scope (in)
1. Zoom, scroll and selection work on the partially imported document while the import runs, bounded by the portion imported so far (define the bound from the spec and say what you chose).
2. Overlays that belong to the *previous* document (markers, selection, loop) must not be drawn over the importing one.
3. Interactions must not stall the import or busy-loop — go through the frame scheduler.

## Tests
- Interaction during an import: the view responds, bounds are respected, stale overlays are gone.
- The import still completes with the same result.

`just check` must pass.

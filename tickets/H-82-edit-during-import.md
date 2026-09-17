# H-82 — Editing during an import

- **Tier:** Sonnet (small; confirm the premise first)
- **From:** H-76's open question. SPEC-005 §2.3 item 4 lists *editing* as disabled while an import runs, but Cut/Copy/Delete/Trim/Silence/Insert Silence are never gated on `isImporting` — in the Edit menu or the waveform's right-click menu. H-76 left it alone because those commands operate on the **previous** document, which the import never touches until commit, so nothing is corrupted today; it is a UX gap, not a data bug.
- **Read first:** CLAUDE.md, MEMORY.md (H-76's `interactionLenSamples` and what it deliberately left out; H-66's right-click menu and its busy gating of `edit_copy`; H-71's import drawing), specs/SPEC-005 §2.3, `ui/src/lib/waveform/WaveformView.svelte`, the Edit menu, `ui/src/lib/state/`'s job/busy state.

## Scope (in)
1. Decide and state which reading of §2.3 item 4 is right: editing is disabled because the *view* belongs to the importing file, or editing is fine because the open document is untouched. Say which, in the ticket report and in a spec amendment if the spec's wording needs sharpening.
2. Implement that decision consistently across the Edit menu, the right-click menu and the keyboard shortcuts — whichever way it goes, all three must agree.
3. If editing stays enabled, make sure nothing can act on the importing document by mistake.

## Tests
The chosen behaviour in all three entry points, with an import running and with none.

`just check` must pass.

# T-810 — Plugin state in presets and sidecars (audit and gap fill)

- **Tier:** Sonnet (no review loop; blocking findings only)
- **Audit first.** T-803, T-806 and T-807 each added state round-trip tests through the sidecar and module presets. H-29 keeps a Missing slot with its state blob after an uninstall. Build a matrix of each requirement (implemented, partial or missing) and fill only the gaps.
- **Read first:**
  - CLAUDE.md, MEMORY.md (the T-803, T-806, T-807, H-29, T-406 and H-22 entries);
  - docs/adr/ADR-005 (state blobs, versions), ADR-008 (Amendments 3–9), ADR-004 (sidecar, journal), specs/SPEC-012/SPEC-018 (rack state in the sidecar, presets);
  - `crates/presets`, `crates/project` (sidecar), `crates/plugin-host`, `crates/sandbox`.

## Scope (in)
1. **Round trips for every format** (CLAP, VST3, LV2): the sidecar save/reopen, the journal crash-recovery path, module presets, rack presets, bake undo attachments (T-602), and preset export/import files (H-22).
2. **Missing or changed plugins:**
   - A document or preset that references a plugin that isn't installed, is blocklisted, disabled or shadowed keeps its slot and state blob verbatim, and shows the right status.
   - When the plugin comes back (install or rescan), the slot recovers without losing its state.
3. **Version and identity:**
   - a state blob from a different plugin version loads if the plugin accepts it, and otherwise fails gracefully with a notice while keeping the blob;
   - a state blob can never be applied to a different plugin id.
4. **Size and robustness:**
   - large blobs (for example 10 MB) and corrupt blobs: the sidecar stays valid, the atomic write holds, and the UI shows an error, never a crash;
   - document the size limit if one exists.
5. **Dirty state:** a plugin parameter change marks the document dirty, and the state is captured at save time. The T-901 caveat applies: GUI-only state isn't refreshed until T-901. Document it.

## Tests
One or more per gap, using the in-repo test plugins for each format.

`just check` must pass.

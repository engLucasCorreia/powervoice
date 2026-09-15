# H-40 — Live recovery of Missing plugin slots

- **Tier:** Sonnet (no review loop; blocking findings only)
- **Found by:** T-810. A slot whose plugin isn't installed (Missing) keeps its state blob, but it only re-resolves when the document is reopened. If the owner installs the plugin (Install Module… or a rescan), the open document's slot stays Missing.
- **Read first:**
  - CLAUDE.md (RT rules), MEMORY.md (the T-804, T-803 async load, H-29, T-809 and T-810 entries: `PluginCatalog` hot-add via `Registry::upsert` to observed registries, `RackHost::resolve_lenient`, Missing placeholders keep the slot verbatim);
  - `crates/plugin-host/src/catalog.rs`, `crates/rack` (`RackHost`, placeholder resolution, async slot loading), `src-tauri/src/plugins.rs`, the rack UI slot status.

## Scope (in)
1. **Re-resolve after registration.** When the catalog registers or upserts module ids (install, quick or full rescan, unblock, re-enable), the rack re-resolves any placeholder slot whose module id is now available:
   - off the audio thread;
   - through the normal async load path, with the Loading status;
   - using the slot's kept state blob and parameter values;
   - swapped in with the T-103 crossfade/hold.

   If loading fails, the slot stays a placeholder with its blob, and the failure is shown.
2. **Notice:** a toast along the lines of "‹Plugin› is available again — restored in the rack" (i18n; the T-702 lint requires the key).
3. **Undo and dirty state:** the re-resolve is not an undoable edit and doesn't mark the document dirty, because the saved state is unchanged. Document this in the ADR-008 amendment log.
4. The same applies to export, loudness and bake renders started afterwards: they use the restored module.

## Tests
- Open a document whose rack references the test CLAP plugin while it's missing, then install or rescan it: the slot becomes Running with the same state, a notice is emitted, and the document is not dirty.
- A failing re-resolve keeps the placeholder and its blob.
- There's no allocation on the audio thread during the swap (the `no_alloc` harness on the rack path).

`just check` and `just check-cross` must pass.

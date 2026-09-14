# T-804 — Scanner orchestration, cache, blocklist

- **Tier:** Sonnet (no review loop; blocking findings only)
- **Depends on:** T-802, T-803.
- **Read first:**
  - CLAUDE.md;
  - docs/adr/ADR-008 §5 (blocklist policy), §6 (scanning) and Amendment 3 (the T-803 scan design);
  - MEMORY.md (T-801, T-802 and T-803 notes; the `/tmp` quota gotcha; the DTO/Settings literal gotcha);
  - `crates/plugin-host/src/scan.rs`, `crates/sandbox` (`--scan`), `src-tauri/src/plugins.rs` and `src-tauri/src/settings.rs`.

## Goal
Plugin discovery is robust and manageable. It runs in the background, never blocks start-up and never crashes the editor. It remembers plugins that failed and exposes everything the plugin manager UI (T-809) needs.

## Scope (in)
1. **Background scan.**
   - Start-up registers cached plugins immediately, then rescans changed files in the background.
   - Run cores/2 sandbox scans in parallel, with a 30 s timeout each.
   - Emit `plugin_scan_progress` events (done/total and the current file) and a final summary.
   - Newly found plugins join the module registry **without a restart**, so Add Module lists them after the scan.
2. **Format-agnostic.**
   - The scanner takes the format as a parameter (`clap` today). T-806, T-807 and T-808 add `vst3`, `lv2` and `jsfx` paths and backends without changing the orchestration.
   - Per-format standard paths are documented for Linux, macOS and Windows.
3. **Richer scan data** (ADR-008 §6): audio ports / supported layouts (mono, stereo-only → shim) and parameter count.
   - Get these by instantiating in the scan sandbox where the descriptor alone isn't enough.
   - Bump the cache schema.
4. **Blocklist** (ADR-008 §5).
   - Scan crash or timeout → auto-blocklisted, keyed by path + size + mtime + content hash.
   - Persisted next to the scan cache.
   - Cleared when the file changes or the user unblocks it.
   - Blocklisted files are skipped by later scans and are not in the registry.
   - Runtime crashes only **flag** a plugin: a crash counter is persisted, and the user decides.
   - The user can also block a plugin manually.
5. **Settings** (additive `Settings.plugins`): custom folders (added to the scan paths) and disabled plugin ids (hidden from Add Module, still listed in the manager).
   - Existing documents that use a disabled plugin still load it.
6. **Commands for T-809** (thin `src-tauri`, generated types):
   - `plugins_list`: each entry carries id, name, vendor, version, format, path, status (ok / disabled / blocklisted with reason / flagged with crash count / missing), ports and params;
   - `plugins_rescan`, with an optional "full" flag that ignores the cache;
   - `plugins_set_enabled`;
   - `plugins_block` / `plugins_unblock`;
   - `plugins_add_folder` / `plugins_remove_folder`.

## Tests
- Parallel scan of several copies of the test CLAP: all found, and progress events arrive in order.
- `crash-on-scan` and `hang-on-scan` get blocklisted and are not rescanned.
- Touching a blocklisted file's mtime or contents clears its blocklist entry.
- Unblock → rescan → the plugin is registered.
- A runtime crash flags the plugin and increments its count without blocklisting it.
- A custom folder is scanned.
- A disabled plugin is hidden from the registry list but still loads for an existing document.
- A plugin found after a rescan is insertable without a restart.
- The cache schema bump invalidates old caches cleanly.

Clean temp dirs with drop guards (the `/tmp` quota). Keep any UI change to generated types only; the plugin manager UI is T-809.

## Out
The plugin manager UI and "Install module…" (T-809); VST3, LV2 and JSFX backends (T-806 to T-808).

`just check` and `just check-cross` must pass.

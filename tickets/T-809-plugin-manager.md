# T-809 — Plugin manager UI + "Install module…"

- **Tier:** Opus, in the `ui-ux-designer` role (`.claude/agents/ui-ux-designer.md`). No review loop; blocking findings only.
- **Depends on:** T-804. It provides `PluginCatalog`, the `plugins_list`, `plugins_rescan`, `plugins_set_enabled`, `plugins_block`, `plugins_unblock`, `plugins_add_folder` and `plugins_remove_folder` commands, the `plugin_scan_progress` events, and `Settings.plugins`.
- **Read first:**
  - CLAUDE.md and PROMPT.md §2 ("Plugin install" row);
  - docs/adr/ADR-006 (installable module ABI and install locations), ADR-008 §5/§6 and Amendments 3–4;
  - MEMORY.md (the T-803, T-804 and H-26 entries: shared Menu/Popover/Dialog actions, `formatNumber`, `?preview` scenes);
  - `docs/design/design-system.md`;
  - `src-tauri/src/ipc/plugin_commands.rs` and `plugin_dto.rs`.

## Goal
From Effects → "Manage Plugins…" and from Preferences, the owner can see every plugin PowerVoice knows about and manage it.

## Scope (in)
1. **Plugin Manager** (a dialog or panel built from the H-25/H-26 kit):
   - A searchable, sortable list with name, vendor, version, format badge, path, and ports/params.
   - Every plugin has a status: OK, Disabled, Blocklisted (with the reason: crashed, timed out, manual) or Flagged (with the crash count).
   - Actions: enable/disable, block/unblock, rescan (quick, or full ignoring the cache) with a live progress bar from `plugin_scan_progress`, and reveal in the file manager if the platform supports it.
   - Empty, loading and error states.
2. **Custom folders:** add a folder (native folder picker) or remove one. A rescan follows automatically.
3. **"Install module…"** (Effects menu and the manager):
   - Pick a plugin file or bundle (`.clap` for now; the dialog filter includes what the backends support).
   - Copy it into the per-user plugin folder that ADR-006/ADR-008 name, e.g. `~/.clap` on Linux, and document macOS/Windows.
   - Scan just that file, then report the result: added N effects, or failed with a reason that gets blocklisted.
   - If the name collides, confirm before replacing. Never install into system folders.
   - The Rust side stays thin in `src-tauri`, with the logic in `vox-plugin-host`.
4. **A flagged plugin in the rack** (crashed at runtime) shows a small warning affordance that opens the manager on that plugin.
5. **Preview:** add a `?preview&scene=plugins` scene with fixtures in every status, screenshot-clean at 1280×720 and 2126×850.

## Tests
- List rendering with every status.
- Filter and sort.
- Each action calls the right command.
- Scan progress updates.
- The install flow: success, collision confirmation, and failure.
- Rust: the install copy (a temp HOME/XDG, never the real home), collision handling, scanning only the new file.

## Out
Plugin GUIs (T-901), VST3/LV2/JSFX backends (T-806–T-808). The format badge must still handle their values.

`just check` must pass.

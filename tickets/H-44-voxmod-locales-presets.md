# H-44 — `.voxmod` locales and presets

- **Tier:** Sonnet (no review loop)
- **Found by:** T-805 (ADR-006 §7 step 5 isn't done). A package's `locales/` and `presets/` are extracted but unused.
- **Read first:** CLAUDE.md, MEMORY.md (the T-805, T-406, H-22, H-33, T-702 and H-40 entries), docs/adr/ADR-006 (§3 layout, §7 install flow, Amendment 2), `crates/plugin-host/src/voxmod.rs` and `install.rs`, `crates/presets`, `ui/src/lib/i18n/*`.

## Scope (in)
1. **Locales:**
   - At load, merge an installed module's `locales/<lang>.json` into i18n, namespaced to its module and param keys only; a package must never override app keys.
   - Validate the file (JSON, string values, key prefix) and reject a bad one with a notice, without failing the install.
   - Remove the strings on uninstall.
   - The T-702 lint ignores package keys.
2. **Presets:**
   - Index the package's `presets/*.json` as **factory** presets for that module: read-only, shown in the slot's Presets menu and the Manage Presets dialog.
   - Validate them with the `vox-presets` schema and sanitizer; unknown params are handled as they are for imports.
   - Remove them on uninstall.
3. **Example package:** add one locale file and two presets to `crates/voxmod-gain`'s package (`just voxmod`).

## Tests
- Install `voxmod-gain` → its presets are listed as factory presets and apply correctly, and its locale strings appear.
- A malicious locale key (one trying to override an app key) is rejected.
- Uninstall removes both.

`just check` must pass.

# H-34 — Plugin polish (after T-806)

- **Tier:** Sonnet (no review loop)
- **Read first:**
  - CLAUDE.md;
  - MEMORY.md (the T-802, T-806 and H-29 entries);
  - `crates/sandbox/tests/teardown.rs`, `crates/plugin-host/src/{scan,catalog}.rs`, `ui/src/lib/plugins/InstallPluginDialog.svelte`.

## Scope (in)
1. **Flaky teardown test.** `crates/sandbox/tests/teardown.rs::nothing_outlives_its_owner` failed once under full-suite load with "file descriptors leaked: 5 → 4". The count dropped below the baseline because the baseline is taken before the warm-up sandbox's descriptors are released.
   - Take the baseline after the warm-up has settled, using a bounded poll until the fd count is stable, not a sleep.
   - Keep the leak check strict.
   - Run the test 20 times in a loop, under parallel load (`cargo test` of the whole sandbox crate), to show it's stable.
2. **Install wording on Linux.** The Install Module file dialog can't select a `.vst3` bundle directory there, so the user picks a file inside the bundle. Explain this in the dialog: a hint line under the picker, and i18n'd dialog title and filter names.
3. **Scan cache name.** Rename `clap-scan.json` (it now holds both formats) to `plugin-scan.json`.
   - Migrate once: read the old file when the new one is missing, then remove the old one.
   - Test the migration.

`just check` and `just check-cross` must pass.

# H-85 — Noise reduction UI remainder (T-504)

- **Tier:** Sonnet
- **From:** the board's T-504 row. S3-06 and H-08 shipped noise capture and the NR module; SPEC-014's UI still lacks the **profile graph**, **Clear Noise Print**, and the **Ctrl+Shift+P** shortcut.
- **Read first:** CLAUDE.md, MEMORY.md (S3-06/H-08's capture flow and the noise-print state blob; H-42's analyzer and its curve drawing; H-63/H-77's graph components; T-701's shortcut registry and the shortcuts doc check in `just check`), specs/SPEC-014 (§2 the profile graph at line ~224 and ~831, Clear Noise Print at ~129 and ~813, and the §4 UI list), `ui/src/lib/rack/`, the NR module's state blob.

## Scope (in)
1. **Profile graph**: the captured print, the reduced-to line, and the rack-output analyzer, as SPEC-014 describes them, in the established graph style.
2. **Clear Noise Print** in the slot menu: drops the blob with the spec's exact consequences (not undoable — say so in the UI the way the spec requires).
3. **Ctrl+Shift+P** through the shortcut registry, so `docs/shortcuts.md` regenerates and the shortcuts check stays green.

## Tests
The graph against a known print; Clear Noise Print's effect on module state and on what the panel shows; the shortcut's registration and action. A screenshot of the NR panel with a captured print, Dark and Light, into the session scratchpad `h85/`.

`just check` must pass.

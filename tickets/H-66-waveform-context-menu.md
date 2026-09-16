# H-66 — Waveform right-click menu, and gate `edit_copy` during jobs

- **Tier:** Sonnet (no review loop)
- **From:** H-56. SPEC-008 §2.11 says the seven edit operations appear on the waveform's context menu; S2-01 never built one, so they are menu-only. H-56 also found `edit_copy` is the one edit command not gated against a running normalize or paste job.
- **Read first:** CLAUDE.md, MEMORY.md (H-26's shared `Menu`/`Popover` — every menu goes through it, opened at the pointer like the Record right-click menu; T-701's shortcuts registry — the chips must match; H-56's edit ops and paste job; H-57's marker drag and hit-testing on the waveform — the context menu must not fight it; T-709's `data-tour` anchors; T-702 i18n lint), specs/SPEC-008 (§2.11 and the op list), `ui/src/lib/waveform/WaveformView.svelte`, `ui/src/lib/edit/EditMenu.svelte`, `src-tauri/src/document.rs`.

## Scope (in)
1. A right-click menu on the waveform with the SPEC-008 §2.11 items (Cut, Copy, Paste, Delete, Trim, Silence, Insert Silence), each enabled and disabled exactly as the Edit menu's copy is, with the same shortcut chips from the registry.
2. It opens at the pointer, closes on outside click and Escape, and is keyboard reachable (the context-menu key, and Shift+F10 where the platform uses it).
3. Right-clicking must not disturb the selection or start a marker drag.
4. Gate `edit_copy` against a running job like its six siblings.

## Tests
- Menu contents and enablement per state (no document, no selection, empty clipboard, job running).
- It doesn't alter the selection or begin a drag.
- Keyboard opening.
- `edit_copy` refused while a job runs.

`just check` must pass.

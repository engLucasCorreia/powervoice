# T-209 — File menu & dialogs: Open, Save, Save As, downmix dialog, save format, clip prompt, unsaved changes, import progress

- **Milestone / wave:** M2 / W3
- **Tier:** Sonnet
- **Depends on:** T-201, T-202, T-205
- **Spec refs:** SPEC-005 (open/save UX: downmix dialog with Ask/Always mix/Always first channel, notices for WAV variants and dropped metadata, stereo-original save warning, clip prompt "Clip and save" / "Save as 32-bit float" / Cancel, Save on compressed sources acts as Save As WAV 24, import progress + progressive waveform), SPEC-004 (§2.6 modified mark `*`, unsaved-changes prompt Save / Don't Save / Cancel), SPEC-002 (New Recording flow replaces the document after the prompt) · **ADR refs:** ADR-007 Amendment 2 (`tauri-plugin-dialog`), ADR-003

## Goal
The owner can open any supported file, see it appear progressively, and save it in the right format
with every prompt SPEC-005 and SPEC-004 require.

## Scope
**In:** File menu (Open…, Save, Save As…, Close; Open Recent is T-306), native dialogs via `tauri-plugin-dialog` (filters per supported formats), probe → downmix dialog → open flow, save-format selector in Save As (WAV 16/24/32f, FLAC), clip prompt and metadata-dropped notice from T-201 outcomes, title bar with file name + `*`, unsaved-changes prompt on open/new/close/quit, import progress indicator, keyboard shortcuts via the keymap registry (Ctrl+O, Ctrl+S, Ctrl+Shift+S — verify Audition defaults or mark provisional for SPEC-019), i18n for every string.

**Out:** recent files (T-306), export (M6).

## Acceptance tests to write
- [ ] Vitest (mockIPC) for each flow: open with downmix choice remembered; save outcomes → prompts; unsaved-changes prompt paths; compressed source Save → Save As with WAV 24 preselected.
- [ ] Manual smoke checklist appended to `docs/checkpoints/M2-smoke.md`.

## Definition of Done
- [ ] `just check` green; report in CLAUDE.md format.

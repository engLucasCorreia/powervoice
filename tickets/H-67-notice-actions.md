# H-67 — Notice actions ("Go to first dropout")

- **Tier:** Sonnet (no review loop; blocking findings only)
- **From:** H-59's audit. SPEC-002 AC-7 wants the dropout notice to offer "Go to first"; no notice anywhere has an action affordance, and `recording.rs` documents the deviation in a comment.
- **Read first:** CLAUDE.md, MEMORY.md (H-59 — `Notice.auto_dismiss_ms` was added the same way, and a new field on the Notice DTO touched 15 hand-written TS literals, so use the H-18 factories; T-702's i18n lint; H-26's Button/Menu kit and dialog action roles; T-708 theme tokens only; H-57's `jumpToMarker` vs `activateMarker` distinction), specs/SPEC-002 (AC-7 and the notice table), `src-tauri/src/{recording,ipc/events}.rs`, the Banner and Toast components, `ui/src/lib/markers/markers.svelte.ts`.

## Scope (in)
1. `Notice.action`: an optional action carrying an i18n label and an identifier the UI can dispatch (keep it a closed set the UI knows how to handle — don't ship arbitrary callbacks over IPC).
2. Banner and Toast render the action as a button, keyboard reachable, using the existing kit and tokens; a notice without an action looks exactly as it does today.
3. First user: the dropout notice offers "Go to first", which jumps to the first dropout marker (navigation, so it must not change the time selection — H-57's `jumpToMarker`).
4. Remove the deviation comment in `recording.rs` once it's true.
5. Check the other notices in SPEC-002's table for actions the spec already asks for, and wire any that are trivial; list the rest in your report.

## Tests
- The DTO round trip, the button's presence and absence, keyboard access, and the jump behaviour (including that the selection is untouched).
- SPEC-002 AC-7.

`just check` must pass.

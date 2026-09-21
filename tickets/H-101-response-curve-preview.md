# H-101 — No way to preview a filter that isn't in the rack yet

- **Tier:** Sonnet
- **From:** H-92. The Explain My Voice modal was specified to draw a dashed EQ-suggestion curve over the measured spectrum. It doesn't, and the reason is structural rather than an oversight.

## The wall
`rack_response_curve` evaluates **an existing rack slot's current parameter values**. There is no way
to ask "what would this filter look like?" without either:
- mutating the user's real rack just to draw a preview — a visible side effect nobody asked for; or
- evaluating biquads in TypeScript, which **SPEC-015 AC-17 forbids** ("the UI never evaluates a
  filter; it only maps (frequency, dB) pairs to pixels"), and which would grow a second filter
  implementation that drifts from the real one.

H-92 escalated instead of picking either. That was the right call and I confirmed it.

- **Read first:** CLAUDE.md, MEMORY.md (the H-92 entry), specs/SPEC-015 §2.6.3 and AC-17, `src-tauri/src/ipc/rack_commands.rs`'s `rack_response_curve`, `crates/rack/`'s response-curve path, `ui/src/lib/analyzer/eqSuggest.ts` (which already produces the `EqAction[]` to preview).

## Scope (in)
1. A backend way to evaluate a response curve from **parameters alone**, with no rack slot and no
   mutation — the same DSP the real path uses, so a preview and the applied result can never
   disagree. Decide whether that is a new command or a mode of the existing one, and say why.
2. Draw the dashed suggestion curve in the Explain modal from that, over — never modifying — the
   measured spectrum, behind the existing "EQ Advice" toggle.
3. Record the decision as a SPEC-015 amendment: AC-17 stands (the UI still evaluates nothing), and
   the preview is served by the backend.

## Out
Any change to what is suggested — H-94 owns that, and its suggestions are deliberately conservative.

## Tests
The preview curve matches what the applied filter actually produces (the point of doing it this
way); the toggle; no rack mutation when previewing.

`just check` must pass.

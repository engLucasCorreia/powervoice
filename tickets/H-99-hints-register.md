# H-99 — Two voices now exist for the same measurements

- **Tier:** Sonnet (small, but it is about what the app tells people)
- **From:** H-94. The diagnostics panel's hints (`diagnosticsHints.ts`, from H-42) and the new Explain My Voice prose describe the *same* measurements with different verdicts:
  - the panel says "Dull — presence is lacking" and offers an **air boost**; the new layer never suggests boosting air to flatten a voice, because a voice is not supposed to be flat;
  - the panel calls elevated upper mids "Forward or harsh" at *warn*; the new layer uses "harsh" only on substantial evidence, and says nothing at all when a threshold is crossed by less than 1 dB.
- **Read first:** CLAUDE.md, MEMORY.md (the H-94 entry, which states the principles), `ui/src/lib/analyzer/diagnosticsHints.ts`, `ui/src/lib/analyzer/explain/prose.ts`, specs/SPEC-007 §8.10.

## Scope (in)
1. Decide which register is right — H-94's, in my view, since it was written against these principles deliberately — and bring the older hints into line.
2. Keep the panel's hints terse: they are one-liners next to a number, not the modal's paragraphs. Matching *principles* does not mean matching *length*.
3. If SPEC-007 §8.10's thresholds themselves encode a verdict that no longer holds (the air-boost suggestion in particular), amend the spec rather than diverging from it silently.

## Tests
The panel's existing hint tests, updated; an assertion that no hint recommends boosting air.

`just check` must pass.

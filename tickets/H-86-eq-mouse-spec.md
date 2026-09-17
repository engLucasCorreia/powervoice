# H-86 — EQ mouse gestures should match the spec (and the keyboard)

- **Tier:** Sonnet (small)
- **From:** H-84's open question. Two divergences from SPEC-015 §2.6.4, both predating H-84:
  1. **Double-click toggles a band's enable state**, but the spec says it **resets the band to its defaults**. S3-07 implemented the toggle as a deliberate lean-slice shortcut and recorded it. Now that H-84's keyboard Home *does* reset per the spec, the same graph answers the same intent two different ways depending on the input device.
  2. **The wheel doesn't step HP/LP slope**, which §2.6.4 names for the wheel as well as the keyboard.
- **Read first:** CLAUDE.md, MEMORY.md (S3-07's recorded deviation and H-84's entry), specs/SPEC-015 §2.6.4, `ui/src/lib/eq/EqGraph.svelte`, `ui/src/lib/eq/drag.ts`, `ui/src/lib/eq/keyboardNav.ts` (H-84's step maths — reuse it, don't re-derive).

## Scope (in)
1. Make double-click reset the band to its defaults, as the spec says and as Home already does.
2. If toggling a band by mouse is still wanted, give it a gesture the spec leaves free (or the existing enable control) — say what you chose and why.
3. Make the wheel step HP/LP slope.
4. Remove the "deferred" notes in `EqGraph.svelte` that these items leave behind, so the file stops advertising a gap that no longer exists.

## Tests
Double-click resets (not toggles); the wheel steps slope; the keyboard equivalents still behave as H-84 left them.

`just check` must pass.

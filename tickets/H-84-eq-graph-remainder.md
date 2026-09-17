# H-84 — EQ graph remainder (T-409)

- **Tier:** Sonnet
- **From:** the board's T-409 row. S3-07 shipped the EQ graph; three pieces of SPEC-015 §2.6 were never built: the **live spectrum overlay** behind the curve, **keyboard-operable nodes**, and the **expanded view**.
- **Read first:** CLAUDE.md, MEMORY.md (S3-07's EQ graph; H-42's analyzer work and its Spectrum Inspector; H-43's frame scheduler — the overlay must not reintroduce a perpetual redraw; H-26's kit and T-708's tokens; H-63/H-77's transfer graph, which solved the same "draggable handles + readable labels" problem and is the house style to match), specs/SPEC-015 (§2.6 including §2.6.5, AC-19, AC-21), specs/SPEC-007 AC-20, the EQ graph component and `ui/src/lib/transfer/`.

## Scope (in)
1. **Spectrum overlay** behind the EQ curve, on the fixed −90…0 dBFS scale SPEC-015 §113 names, fed by the existing analyzer tap. It must be toggleable and must not keep the scheduler awake when the analyzer is idle.
2. **Keyboard nodes** (SPEC-015 §2.6.5, AC-19): select a band, move frequency/gain/Q with the arrow keys and the documented modifiers, with correct ARIA and a visible focus ring. Keyboard-only operation must be genuinely usable, not nominally present.
3. **Expanded view**: the larger EQ graph the spec describes, reachable the way the spec says (and consistent with how the Spectrum Inspector opens).

## Tests
AC-19 and AC-21; the overlay's idle behaviour (no frames when nothing changes); keyboard editing of each parameter. Screenshots of the normal and expanded views in Dark and Light into the session scratchpad `h84/`.

`just check` must pass.

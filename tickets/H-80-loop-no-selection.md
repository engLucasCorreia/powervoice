# H-80 — Loop the whole document when nothing is selected (owner-requested feature)

- **Tier:** Sonnet
- **Requested by the owner while using the app:** loop playback *works* when part of the waveform is
  selected. With no selection, playback runs to the end and stops. The owner wants it to loop the
  whole waveform, start to end, instead.

**This is a feature, not a bug.** The current behaviour is exactly what SPEC-003 §2.2 specifies
("inert with no time selection"), and `Transport::loop_region()`
(`crates/engine/src/transport.rs`) returns `None` in that case by design. Nothing is broken — the
spec's decision is being revised. Do not go looking for a defect in the selection loop path; it
works, and it must keep working unchanged.

- **Read first:** CLAUDE.md, MEMORY.md (H-37's loop implementation and A-023's seamless seam; the transport/reader packet path), specs/SPEC-003 (§2.2, §3 parameter table, §4's loop-wrap packet rules, AC-4, Amendment 1), `crates/engine/src/transport.rs`, `crates/engine/src/reader.rs`, the transport UI and `ui/src/lib/transport/`.

## Scope (in)
1. **Loop with no selection loops the whole document** (0..len), with the same seamless, sample-exact
   seam Amendment 1 guarantees for a selection loop.
2. Write it up as a new SPEC-003 amendment — §2.2, the §3 table's "inert with no time selection"
   note, and AC-4's wording — the way earlier tickets amended specs.
3. Making a selection while looping the whole document switches the loop to that selection;
   clearing the selection goes back to the whole document. No stop, no glitch at the switch.
4. A selection shorter than the minimum loop length currently falls back to nothing at all. Decide
   between falling back to the whole document and showing the standard notice, and say why.
5. The transport button's on-state must always mean "this is actually looping".

## Tests
- Loop on, no selection: ≥ 3 passes over the whole document, sample-exact, no stop at the end.
- Loop on, with a selection: still AC-4 exact — this is the regression test that protects the
  behaviour the owner confirmed works.
- Selecting and deselecting while looping.
- Loop toggled on during the final second of playback: it wraps rather than stopping.
- Selection under the minimum length: the chosen behaviour.

`just check` must pass.

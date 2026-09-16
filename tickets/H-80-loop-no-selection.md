# H-80 — Loop Playback doesn't loop (owner-reported)

- **Tier:** Sonnet
- **Reported by the owner while using the app:** "When I select the loop playback function, it does not loop — when the audio finishes it stops, even though loop playback is active."

## Likely cause, to confirm first
SPEC-003 §2.2 says loop is **inert with no time selection**, and `Transport::loop_region()`
(`crates/engine/src/transport.rs`) returns `None` both when there is no selection and when the
selection is shorter than `min_len`. So enabling Loop with nothing selected silently does nothing —
playback runs to the end of the document and stops, which is exactly what the owner saw. That
behaviour is wrong for a voice-over editor and wrong next to Audition, where Loop Playback loops the
**whole file** when there is no selection.

**Confirm the diagnosis before changing anything** (reproduce with a test through the fake backend,
both with and without a selection). If looping is *also* broken with a selection, that is a second
bug — fix it and say so.

- **Read first:** CLAUDE.md, MEMORY.md (H-37's loop implementation and A-023's seamless seam; the transport/reader packet path), specs/SPEC-003 (§2.2, §3 parameter table, §4's loop-wrap packet rules, AC-4, Amendment 1), `crates/engine/src/transport.rs`, `crates/engine/src/reader.rs`, the transport UI and `ui/src/lib/transport/`.

## Scope (in)
1. **Loop with no selection loops the whole document** (0..len). Write it up as a new SPEC-003
   amendment — §2.2, the §3 table's "inert with no time selection" note, and AC-4's wording — the way
   earlier tickets amended specs, and add the acceptance test.
2. Make a selection shorter than the minimum loop length **visible** rather than silently inert:
   either fall back to the whole document or show the standard notice. Choose, and say why.
3. Keep Amendment 1's guarantees: the seam stays sample-exact and seamless, the playhead wraps, and
   turning Loop off mid-pass still finishes the pass.
4. The transport button's on-state must mean "this is actually looping".

## Tests
- Loop on, no selection: ≥ 3 passes over the whole document, sample-exact, no stop at the end.
- Loop on, with a selection: still AC-4 exact (regression).
- Loop toggled on during the final second of playback: it wraps rather than stopping.
- Selection under the minimum length: the chosen behaviour.

`just check` must pass.

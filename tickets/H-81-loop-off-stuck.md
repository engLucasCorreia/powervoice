# H-81 — Turning Loop off doesn't stop the loop, and the loop strip never goes away (owner-reported)

- **Tier:** Sonnet. **This is a real defect** — reproduce it before changing anything.
- **Reported by the owner while using the app:** "I selected part of the audio, activated loop, and
  even though I deactivated it, it still loops where I activated it. There is a pink bar at the top
  that stays there always, similar to where it was looping."

Two symptoms, possibly one cause:
1. **Playback keeps looping the old region after Loop is switched off.** SPEC-003 §2.1/§2.2 say the
   pass being played finishes and then playback stops at the old loop end — it must not keep
   wrapping.
2. **The loop overlay stays on screen.** The "pink bar at the top" is the loop brace strip
   (`--wave-loop: #c9a0ff`, `LOOP_STRIP_PX`), drawn by `ui/src/lib/render/loopOverlay.ts` from
   `transport.state.loop_range`. It should disappear the moment the effective loop region becomes
   `None`.

## Where to look
The chain is: `toggleLoop` (`ui/src/lib/state/transport.svelte.ts`) → `transport_set_loop`
(`src-tauri/src/ipc/audio_commands.rs`) → `Control::set_loop`
(`crates/engine/src/control.rs:812`) → `sync_loop(finish)` → the reader (`crates/engine/src/reader.rs`,
the `finish` / END-packet path) and the output callback. `Control::loop_region()` feeds both the
reader and the `loop_range` the UI draws, and `Transport::loop_region()` already returns `None` when
the toggle is off — so on paper this works, which means the actual break is somewhere else. Find it;
don't trust the reading above. Suspects worth checking: whether `sync_loop`'s early return
(`region == self.loop_sent`) or its `finish` computation can skip the update; whether the output
callback latches a region at play start; whether the returned `TransportState` actually reaches the
store and re-renders the overlay; and whether the strip is drawn from stale state after the toggle.

- **Read first:** CLAUDE.md, MEMORY.md (H-37's loop implementation and A-023; H-43's frame scheduler and its draw-on-demand rule — a stale overlay can also be a missed invalidation), specs/SPEC-003 (§2.1, §2.2, §3, AC-4, Amendment 1), specs/SPEC-006 §2.12's loop amendment.

## Scope (in)
1. Reproduce both symptoms in tests — an engine-level test through the fake backend for the
   playback half, a UI test for the overlay half. **State the real root cause in your report.**
2. Fix it so that turning Loop off finishes the current pass and then plays on past the old loop end
   (or stops there) exactly as SPEC-003 §2.2 describes, and the strip and boundary lines vanish as
   soon as the loop is off.
3. Check the same path for Stop, Pause, clearing the selection, and closing the document — any of
   these leaving a stale loop region or a stale strip is the same bug.
4. Check the spectral pane too; it draws the same overlay.

## Tests
As above, plus a regression test that the *working* case stays working: loop on with a selection
still loops sample-exactly (AC-4).

`just check` must pass.

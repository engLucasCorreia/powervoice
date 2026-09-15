/**
 * Every shortcut action id (T-104; the registry consolidated at T-701). This is the single source
 * of truth for what a key binding can dispatch — features register handlers against these ids
 * later (transport, recording, markers, history); an action with no registered handler is simply
 * a no-op (ticket: "unknown/unhandled actions are no-ops").
 */
export type ActionId =
  | "transport.play_pause"
  | "transport.play_from_start"
  | "transport.return_to_start"
  // H-37 (SPEC-003 §2.1/§2.5 amendment): the Loop playback toggle.
  | "transport.toggle_loop"
  | "record.toggle"
  | "marker.add"
  | "marker.delete_selected"
  | "marker.next"
  | "marker.prev"
  | "history.undo"
  | "history.redo"
  | "file.open"
  | "file.save"
  | "file.save_as"
  | "waveform.zoom_in"
  | "waveform.zoom_out"
  // H-35 (SPEC-006 §2.6): "Zoom to selection" and "Zoom full" — both commands exist (menu +
  // toolbar per the spec), but §2.6 defers their keyboard binding to SPEC-019 (two sources
  // disagree, neither checkable against the 403'ing official page), so these have no `registry.ts`
  // entry — only `dispatchAction` calls from the menu/toolbar `onselect`/`onclick`.
  | "waveform.zoom_to_selection"
  | "waveform.zoom_full"
  // H-35 (SPEC-006 §2.4/§2.6): vertical (amplitude) zoom — `Alt+=`/`Alt+-` verified against
  // Audition (tutorialtactic.com, pie-menu.com); `Alt+0` to reset is a PowerVoice-original
  // conservative choice (SPEC-006 doesn't name a reset binding) — see `registry.ts`.
  | "waveform.zoom_in_vertical"
  | "waveform.zoom_out_vertical"
  | "waveform.zoom_reset_vertical"
  | "edit.cut"
  | "edit.copy"
  | "edit.paste"
  | "edit.delete"
  | "edit.trim"
  | "waveform.select_all"
  | "waveform.deselect"
  | "nr.capture_noise_print"
  | "spectral.toggle"
  // T-701 / A-020: keyboard nudge (move the cursor, or the whole selection, by one step) and
  // extend (grow the selection by one step from the given edge) — no spec named a binding, no
  // Audition default was found in the available sources (helpx.adobe.com 403s; tutorialtactic,
  // killerkeys, prismmultimedia and defkey were checked and none document plain-arrow or
  // Shift+arrow selection commands for the Waveform Editor), so these are PowerVoice-original,
  // conservative, non-conflicting bindings — see `registry.ts` for the exact key mapping.
  | "selection.nudge_left"
  | "selection.nudge_right"
  | "selection.extend_left"
  | "selection.extend_right";

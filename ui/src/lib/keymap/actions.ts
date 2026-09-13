/**
 * Every keymap action id (T-104). This is the single source of truth for what a key binding can
 * dispatch — features register handlers against these ids later (transport, recording, markers,
 * history); an action with no registered handler is simply a no-op (ticket: "unknown/unhandled
 * actions are no-ops").
 */
export type ActionId =
  | "transport.play_pause"
  | "transport.play_from_start"
  | "transport.return_to_start"
  | "record.toggle"
  | "marker.add"
  | "history.undo"
  | "history.redo"
  | "file.open"
  | "file.save"
  | "file.save_as"
  | "waveform.zoom_in"
  | "waveform.zoom_out";

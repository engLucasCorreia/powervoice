import type { ActionId } from "./actions";

/**
 * One default key binding. `code` is a `KeyboardEvent.code` (physical key, layout-independent —
 * matches how ADR-009's spike input log records keys, e.g. `Ctrl+KeyZ`). `mod` means "the
 * platform's primary modifier": Ctrl on Windows/Linux, ⌘ on macOS (`isPlatformMac` in
 * `registry.ts` resolves which). `shift` is a plain, non-platform-dependent modifier.
 *
 * No remapping UI in v1 (ticket) — this table is the only source of bindings.
 */
export interface KeyBinding {
  action: ActionId;
  code: string;
  /** Requires the platform's primary modifier (Ctrl / ⌘). Defaults to not required. */
  mod?: boolean;
  /** Requires Shift. Defaults to not required. */
  shift?: boolean;
}

/**
 * Default keymap (PROMPT §3.6, SPEC-002 §2.2, SPEC-003 §2.5, SPEC-004 §2.2, SPEC-005 §2.1,
 * SPEC-006 §2.6; MEMORY D-014):
 * - Space = play/pause
 * - Shift+Space = play from start (owner-confirmed; not Record — D-014)
 * - Home = return to start
 * - Shift+R = record toggle (**provisional** — SPEC-002 §2.2, final binding in SPEC-019)
 * - M = add marker
 * - Ctrl/⌘+Z = undo
 * - Ctrl/⌘+Shift+Z = redo
 * - Ctrl/⌘+O = open, Ctrl/⌘+S = save, Ctrl/⌘+Shift+S = save as (S1-03, SPEC-005 §2.1)
 * - `=`/`-` = waveform zoom in/out (S1-03, SPEC-006 §2.6; `Alt+=`/`Alt+-` vertical zoom and
 *   Ctrl/Shift+wheel zoom are deferred, out of this ticket's scope)
 */
export const DEFAULT_KEYMAP: readonly KeyBinding[] = [
  { action: "transport.play_pause", code: "Space" },
  { action: "transport.play_from_start", code: "Space", shift: true },
  { action: "transport.return_to_start", code: "Home" },
  { action: "record.toggle", code: "KeyR", shift: true },
  { action: "marker.add", code: "KeyM" },
  { action: "history.undo", code: "KeyZ", mod: true },
  { action: "history.redo", code: "KeyZ", mod: true, shift: true },
  { action: "file.open", code: "KeyO", mod: true },
  { action: "file.save", code: "KeyS", mod: true },
  { action: "file.save_as", code: "KeyS", mod: true, shift: true },
  { action: "waveform.zoom_in", code: "Equal" },
  { action: "waveform.zoom_out", code: "Minus" },
];

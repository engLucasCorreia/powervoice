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
  /** Requires Alt (⌥ on macOS). Defaults to not required (S2-03: Ctrl+Alt+arrow navigation). */
  alt?: boolean;
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
 * - Ctrl/⌘+X/C/V = cut/copy/paste, Delete = delete, Ctrl/⌘+T = trim to selection (Crop); Ctrl/⌘+A
 *   selects all, Esc clears the selection (S2-01, SPEC-006 §2.9, SPEC-008 §2.11). Silence and
 *   Insert Silence have no default binding (menu only, SPEC-008 §2.11).
 * - Ctrl/⌘+0 = delete selected marker(s), Ctrl/⌘+Alt+→/← = next/previous marker (S2-03,
 *   SPEC-009 §2.6/§2.7; Delete All is deferred, ticket "Out" list).
 * - Shift+P = Capture Noise Print (S3-06, SPEC-014 §2.3; **provisional**, per Audition's default
 *   confirmed by two secondary sources — final binding in SPEC-019).
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
  { action: "edit.cut", code: "KeyX", mod: true },
  { action: "edit.copy", code: "KeyC", mod: true },
  { action: "edit.paste", code: "KeyV", mod: true },
  { action: "edit.delete", code: "Delete" },
  { action: "edit.trim", code: "KeyT", mod: true },
  { action: "waveform.select_all", code: "KeyA", mod: true },
  { action: "waveform.deselect", code: "Escape" },
  { action: "marker.delete_selected", code: "Digit0", mod: true },
  { action: "marker.next", code: "ArrowRight", mod: true, alt: true },
  { action: "marker.prev", code: "ArrowLeft", mod: true, alt: true },
  { action: "nr.capture_noise_print", code: "KeyP", shift: true },
];

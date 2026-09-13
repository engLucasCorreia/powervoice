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
 * Default keymap (PROMPT §3.6, SPEC-002 §2.2, SPEC-003 §2.5, SPEC-004 §2.2; MEMORY D-014):
 * - Space = play/pause
 * - Shift+Space = play from start (owner-confirmed; not Record — D-014)
 * - Home = return to start
 * - Shift+R = record toggle (**provisional** — SPEC-002 §2.2, final binding in SPEC-019)
 * - M = add marker
 * - Ctrl/⌘+Z = undo
 * - Ctrl/⌘+Shift+Z = redo
 */
export const DEFAULT_KEYMAP: readonly KeyBinding[] = [
  { action: "transport.play_pause", code: "Space" },
  { action: "transport.play_from_start", code: "Space", shift: true },
  { action: "transport.return_to_start", code: "Home" },
  { action: "record.toggle", code: "KeyR", shift: true },
  { action: "marker.add", code: "KeyM" },
  { action: "history.undo", code: "KeyZ", mod: true },
  { action: "history.redo", code: "KeyZ", mod: true, shift: true },
];

import type { ActionId } from "./actions";
import type { MessageKey } from "../i18n";

/** The subset of `KeyboardEvent` that matching needs — kept minimal so tests can pass a plain
 * object instead of constructing a real `KeyboardEvent`. `key` (the produced character) is
 * optional: every existing test builds a plain `code`-matched event without it, and it's only
 * read for a binding that itself carries `key` (H-64, SPEC-009 §2.4's `/` rename binding). */
export interface KeyEventLike {
  code: string;
  key?: string;
  shiftKey: boolean;
  ctrlKey: boolean;
  metaKey: boolean;
  altKey: boolean;
}

/**
 * T-701: where a binding lives. Two entries in the same scope must never share a binding (a
 * registry test enforces it) — but two entries CAN share a binding across scopes when the scopes
 * never dispatch through the same matcher at the same time:
 * - `"global"` and `"waveform"` both run through the single `attachKeymap` listener/`matchBinding`
 *   table below, and are effectively concurrent (PowerVoice has one window, one waveform view, no
 *   tabs) — so in practice these two must be jointly unique too (`findDuplicateBindings` treats
 *   them as one group, see below). `"waveform"` is for commands that only make sense on the
 *   waveform editing surface (zoom, selection); `"global"` is everything else reachable while the
 *   editor has focus (SPEC-008 §2.11).
 * - `"dialog"` bindings belong to a specific modal dialog and are matched locally by that dialog's
 *   own `onkeydown` (e.g. Escape/Enter) — never through this table — so they may freely reuse a
 *   "global"/"waveform" binding: `listener.ts`'s `isModalDialogOpen()` guard means the global
 *   table never dispatches while a dialog is open anyway. No dialog defines a *custom* shortcut
 *   through the registry today (Escape/Enter are handled ad hoc per dialog), so this scope has no
 *   entries yet — reserved for one that does.
 * - `"text-input"` is the same idea for a binding meant to fire *inside* a text field (native
 *   editing keys aside) — `isEditableTarget` currently blocks the whole table there, so this scope
 *   also has no entries yet (reserved).
 */
export type ShortcutScope = "global" | "waveform" | "dialog" | "text-input";

/** One default key binding: exactly one of `code`/`key` is set.
 * - `code` is a `KeyboardEvent.code` (physical key, layout-independent — matches how ADR-009's
 *   spike input log records keys, e.g. `Ctrl+KeyZ`). Every binding uses this except the one below.
 * - `key` is a `KeyboardEvent.key` (the *produced character*), for a binding SPEC-009 §2.4 says
 *   must work "on layouts where [it] needs Shift" (H-64: `/` = rename marker) — matched directly
 *   against `event.key`, ignoring `shift` (the produced character already reflects it; `mod`/`alt`
 *   still apply normally).
 *
 * `mod` means "the platform's primary modifier": Ctrl on Windows/Linux, ⌘ on macOS
 * (`isPlatformMac` below resolves which — this is how one table expresses "the default binding
 * per platform" (T-701) without duplicating every row). `shift` is a plain, non-platform-dependent
 * modifier (meaningless on a `key` binding, see above).
 *
 * No remapping UI in v1 (ticket) — this table is the only source of bindings.
 */
export interface KeyBinding {
  action: ActionId;
  code?: string;
  key?: string;
  /** Requires the platform's primary modifier (Ctrl / ⌘). Defaults to not required. */
  mod?: boolean;
  /** Requires Shift. Defaults to not required. Ignored for a `key` binding. */
  shift?: boolean;
  /** Requires Alt (⌥ on macOS). Defaults to not required (S2-03: Ctrl+Alt+arrow navigation). */
  alt?: boolean;
}

/** T-701: a registry entry — a binding plus the i18n label to show for it (menus, the Help ▸
 * Keyboard Shortcuts dialog, `docs/shortcuts.md`) and its scope. */
export interface ShortcutDef extends KeyBinding {
  scope: ShortcutScope;
  labelKey: MessageKey;
}

/**
 * The shortcut registry (PROMPT §3.6, SPEC-002 §2.2, SPEC-003 §2.5, SPEC-004 §2.2, SPEC-005 §2.1,
 * SPEC-006 §2.6/§2.9/§2.10, SPEC-007 §2.1, SPEC-008 §2.11, SPEC-009 §2.6/§2.7, SPEC-014 §2.3;
 * MEMORY D-014; T-701 shortcut audit, A-020): the single source of truth for every default
 * binding, used by the global handler (`listener.ts`), every menu's `Kbd` chip
 * (`shortcutLabelForAction`), the Help ▸ Keyboard Shortcuts dialog, and `docs/shortcuts.md`
 * (`scripts/docs/generate_shortcuts.mjs`).
 *
 * - Space = play/pause; Shift+Space = play from start (owner-confirmed at the M0 checkpoint —
 *   Audition sources disagree and some claim Shift+Space is Record, but the owner's own Audition
 *   muscle memory settled this; SPEC-003 §2.5 documents the contradiction). Home = return to
 *   start. Shift+R = record toggle (**provisional**: T-701 re-searched Adobe Audition's default
 *   for Record — helpx.adobe.com/audition/.../default-keyboard-shortcuts.html still 403s to
 *   automated fetch, and the only other source found (tutorialtactic) claims Shift+Space "(in
 *   record mode)", which is the same owner-overridden ambiguity — so Shift+R stays, still
 *   provisional, not contradicted by anything with better sourcing).
 * - Ctrl/⌘+L = Loop playback toggle (H-37, SPEC-003 §2.5 amendment): Audition's default per the
 *   secondary sources checked (killerkeys, Prism Multimedia); helpx.adobe.com still 403s. Ctrl+L
 *   was unbound in PowerVoice.
 * - M = add marker. Ctrl/⌘+Z = undo, Ctrl/⌘+Shift+Z = redo. Ctrl/⌘+O/S/Shift+S = open/save/save as.
 * - `=`/`-` = waveform zoom in/out (horizontal). H-35: `Alt+=`/`Alt+-` = vertical (amplitude) zoom
 *   in/out — **Verified** against Audition by the same two sources as horizontal `=`/`-`
 *   (tutorialtactic.com, pie-menu.com, both agreeing on the modifier per SPEC-006 §2.6). `Alt+0` =
 *   reset vertical zoom to 1× — no source documents a reset binding, so this is a
 *   PowerVoice-original, conservative choice (unused elsewhere, and `0` already reads as
 *   "reset/clear" the same shape as `Ctrl+0` = delete selected marker(s), see SPEC-006 §2.6's
 *   H-35 amendment). Zoom to Selection/Zoom Full exist (menu + toolbar, SPEC-006 §2.6) but their
 *   keyboard binding is still deferred to SPEC-019 (two sources disagree) — no registry entry.
 * - Ctrl/⌘+X/C/V = cut/copy/paste, Delete = delete, Ctrl/⌘+T = trim to selection (Crop); Ctrl/⌘+A
 *   selects all, Esc clears the selection. Silence/Insert Silence and Mix Paste/Copy to New have
 *   no default binding (menu only / not implemented, SPEC-008 §2.11).
 * - Ctrl/⌘+0 = delete selected marker(s), Ctrl/⌘+Alt+→/← = next/previous marker. Delete All
 *   Markers (Ctrl/⌘+Alt+0 in Audition) has no `deleteAllMarkers` command to bind yet (S2-03:
 *   "Delete All ... deferred") — T-701 report flags it rather than inventing the feature.
 * - Shift+P = Capture Noise Print (**provisional**, two secondary sources agree). Shift+D =
 *   show/hide the spectral pane (verified Audition binding).
 * - T-701/A-020: Left/Right Arrow nudge the cursor (or the whole selection, unchanged length, if
 *   one exists); Shift+Left/Right Arrow extend the selection from the edge in that direction
 *   (grow only — see `waveform/selection.ts::extendSelectionEdge`). No Audition default was found
 *   for either in the sources checked (see `actions.ts`) — conservative, PowerVoice-original,
 *   chosen to match the ticket's own example ("Shift+arrow extends") and to avoid every existing
 *   binding above. Zero-crossing snap (`Settings.snap_to_zero_crossing`, T-206) applies to the
 *   extended edge only, not to a plain nudge.
 */
export const SHORTCUTS: readonly ShortcutDef[] = [
  { action: "transport.play_pause", code: "Space", scope: "global", labelKey: "shortcut.transport.play_pause" },
  {
    action: "transport.play_from_start",
    code: "Space",
    shift: true,
    scope: "global",
    labelKey: "shortcut.transport.play_from_start",
  },
  {
    action: "transport.return_to_start",
    code: "Home",
    scope: "global",
    labelKey: "shortcut.transport.return_to_start",
  },
  {
    action: "transport.toggle_loop",
    code: "KeyL",
    mod: true,
    scope: "global",
    labelKey: "shortcut.transport.toggle_loop",
  },
  { action: "record.toggle", code: "KeyR", shift: true, scope: "global", labelKey: "shortcut.record.toggle" },
  { action: "marker.add", code: "KeyM", scope: "global", labelKey: "shortcut.marker.add" },
  { action: "history.undo", code: "KeyZ", mod: true, scope: "global", labelKey: "shortcut.history.undo" },
  {
    action: "history.redo",
    code: "KeyZ",
    mod: true,
    shift: true,
    scope: "global",
    labelKey: "shortcut.history.redo",
  },
  { action: "file.open", code: "KeyO", mod: true, scope: "global", labelKey: "shortcut.file.open" },
  { action: "file.save", code: "KeyS", mod: true, scope: "global", labelKey: "shortcut.file.save" },
  {
    action: "file.save_as",
    code: "KeyS",
    mod: true,
    shift: true,
    scope: "global",
    labelKey: "shortcut.file.save_as",
  },
  { action: "waveform.zoom_in", code: "Equal", scope: "waveform", labelKey: "shortcut.waveform.zoom_in" },
  { action: "waveform.zoom_out", code: "Minus", scope: "waveform", labelKey: "shortcut.waveform.zoom_out" },
  {
    action: "waveform.zoom_in_vertical",
    code: "Equal",
    alt: true,
    scope: "waveform",
    labelKey: "shortcut.waveform.zoom_in_vertical",
  },
  {
    action: "waveform.zoom_out_vertical",
    code: "Minus",
    alt: true,
    scope: "waveform",
    labelKey: "shortcut.waveform.zoom_out_vertical",
  },
  {
    action: "waveform.zoom_reset_vertical",
    code: "Digit0",
    alt: true,
    scope: "waveform",
    labelKey: "shortcut.waveform.zoom_reset_vertical",
  },
  { action: "edit.cut", code: "KeyX", mod: true, scope: "global", labelKey: "shortcut.edit.cut" },
  { action: "edit.copy", code: "KeyC", mod: true, scope: "global", labelKey: "shortcut.edit.copy" },
  { action: "edit.paste", code: "KeyV", mod: true, scope: "global", labelKey: "shortcut.edit.paste" },
  { action: "edit.delete", code: "Delete", scope: "global", labelKey: "shortcut.edit.delete" },
  { action: "edit.trim", code: "KeyT", mod: true, scope: "global", labelKey: "shortcut.edit.trim" },
  {
    action: "waveform.select_all",
    code: "KeyA",
    mod: true,
    scope: "waveform",
    labelKey: "shortcut.waveform.select_all",
  },
  { action: "waveform.deselect", code: "Escape", scope: "waveform", labelKey: "shortcut.waveform.deselect" },
  {
    // H-64 (SPEC-009 §2.4): matches the produced character (`key: "/"`), not a physical code, so
    // it still works on a layout where `/` needs Shift — see `KeyBinding`'s doc comment.
    action: "marker.rename",
    key: "/",
    scope: "global",
    labelKey: "shortcut.marker.rename",
  },
  {
    action: "marker.delete_selected",
    code: "Digit0",
    mod: true,
    scope: "global",
    labelKey: "shortcut.marker.delete_selected",
  },
  {
    // H-64 (SPEC-009 §2.6): Delete All Markers.
    action: "marker.delete_all",
    code: "Digit0",
    mod: true,
    alt: true,
    scope: "global",
    labelKey: "shortcut.marker.delete_all",
  },
  {
    action: "marker.next",
    code: "ArrowRight",
    mod: true,
    alt: true,
    scope: "global",
    labelKey: "shortcut.marker.next",
  },
  {
    action: "marker.prev",
    code: "ArrowLeft",
    mod: true,
    alt: true,
    scope: "global",
    labelKey: "shortcut.marker.prev",
  },
  {
    action: "nr.capture_noise_print",
    code: "KeyP",
    shift: true,
    scope: "global",
    labelKey: "shortcut.nr.capture_noise_print",
  },
  {
    // H-85 (SPEC-014 §2.3 "Decided"): shows the Noise Reduction panel — secondary sources give
    // this as Audition's "open Noise Reduction effect" key, provisional for SPEC-019.
    action: "nr.show_panel",
    code: "KeyP",
    mod: true,
    shift: true,
    scope: "global",
    labelKey: "shortcut.nr.show_panel",
  },
  { action: "spectral.toggle", code: "KeyD", shift: true, scope: "global", labelKey: "shortcut.spectral.toggle" },
  {
    action: "selection.nudge_left",
    code: "ArrowLeft",
    scope: "waveform",
    labelKey: "shortcut.selection.nudge_left",
  },
  {
    action: "selection.nudge_right",
    code: "ArrowRight",
    scope: "waveform",
    labelKey: "shortcut.selection.nudge_right",
  },
  {
    action: "selection.extend_left",
    code: "ArrowLeft",
    shift: true,
    scope: "waveform",
    labelKey: "shortcut.selection.extend_left",
  },
  {
    action: "selection.extend_right",
    code: "ArrowRight",
    shift: true,
    scope: "waveform",
    labelKey: "shortcut.selection.extend_right",
  },
  // H-107: F1 opens the in-app Help Centre — the conventional "help" key across desktop platforms,
  // and unbound in every Audition source consulted so far, so it introduces no conflict.
  { action: "help.open_centre", code: "F1", scope: "global", labelKey: "shortcut.help.open_centre" },
];

/**
 * Platform-aware Ctrl ↔ ⌘ (ticket requirement): true when the given platform string looks like
 * macOS. Defaults to `navigator.platform`, but callers (and tests) can pass an explicit string —
 * `navigator.platform` is deprecated but still the simplest available signal, and PowerVoice only
 * needs a mac/non-mac split, not a fully general platform detector.
 */
export function isPlatformMac(platform: string = getNavigatorPlatform()): boolean {
  return /mac/i.test(platform);
}

function getNavigatorPlatform(): string {
  if (typeof navigator === "undefined") {
    return "";
  }
  return navigator.platform ?? "";
}

/**
 * Resolves a key event to an action id, or `null` if no default binding matches. `isMac` decides
 * which physical modifier satisfies a binding's `mod: true` (⌘ on macOS, Ctrl elsewhere) — the
 * *other* platform's primary modifier must NOT be held, so a stray ⌘ on Windows/Linux (or Ctrl on
 * macOS) doesn't accidentally satisfy a binding. A binding requires Alt held or not held exactly
 * as its own `alt` flag says (default: not held) — S2-03's Ctrl+Alt+arrow navigation is the only
 * binding that opts into `alt: true`; every other binding still never matches with Alt held.
 */
export function matchBinding(
  event: KeyEventLike,
  isMac: boolean,
  entries: readonly KeyBinding[] = SHORTCUTS,
): ActionId | null {
  const modPressed = isMac ? event.metaKey : event.ctrlKey;
  const otherModPressed = isMac ? event.ctrlKey : event.metaKey;

  if (otherModPressed) {
    return null;
  }

  for (const binding of entries) {
    if (binding.key !== undefined) {
      // H-64 (SPEC-009 §2.4): matched on the produced character, not the physical key, so it
      // still works on a layout where it needs Shift — `shift` is deliberately not checked here.
      if (
        binding.key === event.key &&
        Boolean(binding.mod) === modPressed &&
        Boolean(binding.alt) === event.altKey
      ) {
        return binding.action;
      }
      continue;
    }
    if (
      binding.code === event.code &&
      Boolean(binding.shift) === event.shiftKey &&
      Boolean(binding.mod) === modPressed &&
      Boolean(binding.alt) === event.altKey
    ) {
      return binding.action;
    }
  }
  return null;
}

/** A binding's unique key: (code, shift, mod, alt) for a `code` binding, or (key, mod, alt) for a
 * `key` one (H-64) — the two schemes never collide since a real `KeyboardEvent.code` and `.key`
 * value never look alike (`"KeyM"` vs. `"/"`). Two default bindings must never share one within
 * the same {@link dispatchGroup}. */
function bindingIdentity(binding: KeyBinding): string {
  if (binding.key !== undefined) {
    return `key=${binding.key}|mod=${Boolean(binding.mod)}|alt=${Boolean(binding.alt)}`;
  }
  return `${binding.code}|shift=${Boolean(binding.shift)}|mod=${Boolean(binding.mod)}|alt=${Boolean(binding.alt)}`;
}

/** "global" and "waveform" dispatch through the same single `matchBinding` table concurrently
 * (see {@link ShortcutScope}'s doc comment) — group them together so a collision between the two
 * is caught, not just a collision within one of them. "dialog"/"text-input" bindings are matched
 * locally by their own component, never through this table, so they get their own group(s). */
function dispatchGroup(scope: ShortcutScope): string {
  return scope === "global" || scope === "waveform" ? "shared" : scope;
}

/** Every duplicated (dispatch group, code, shift, mod, alt) combination. Empty when every group's
 * bindings are unique (T-701: "no two commands share a binding in the same scope", plus the
 * "global"/"waveform" concurrency rule above). */
export function findDuplicateBindings(entries: readonly ShortcutDef[] = SHORTCUTS): string[] {
  const seen = new Map<string, number>();
  for (const binding of entries) {
    const identity = `${dispatchGroup(binding.scope)}|${bindingIdentity(binding)}`;
    seen.set(identity, (seen.get(identity) ?? 0) + 1);
  }
  return [...seen.entries()].filter(([, count]) => count > 1).map(([identity]) => identity);
}

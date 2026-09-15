/**
 * The shared menu's item model and its pure keyboard logic (H-26, WAI-ARIA APG menu/menubar).
 * `Menu.svelte` renders a `MenuEntry[]`; every menu in the app (menu bar, Normalize ▾, Add
 * module, rack slot and preset menus, the Record context menu) describes itself with these.
 */
import type { Snippet } from "svelte";
import type { IconName } from "./icons";

interface EntryBase {
  /** Stable key within its menu. */
  id: string;
  testid?: string;
}

interface ActionBase extends EntryBase {
  label: string;
  disabled?: boolean;
  /** Tooltip (`title`) for the row. */
  title?: string;
  onselect: () => void;
  /** Run `onselect` without closing the menu (steps inside a submenu, e.g. "Save as…" → form). */
  keepOpen?: boolean;
}

export interface MenuItemEntry extends ActionBase {
  kind: "item";
  /** Display shortcut from `shortcutLabelForAction` — drawn as a right-aligned `Kbd` chip. */
  shortcut?: string | null;
  icon?: IconName;
  /** Dim the label without disabling the row (a missing Recent Files entry). */
  muted?: boolean;
  /** Extra attributes for the row (e.g. `data-module-id`). */
  attrs?: Record<string, string>;
  /** A secondary icon action at the row's end (delete a preset); the Delete key also runs it. */
  trailing?: { icon: IconName; label: string; testid?: string; onselect: () => void };
}

export interface MenuCheckboxEntry extends ActionBase {
  kind: "checkbox";
  checked: boolean;
  shortcut?: string | null;
}

export interface MenuRadioEntry extends ActionBase {
  kind: "radio";
  checked: boolean;
  shortcut?: string | null;
}

export interface MenuSubmenuEntry extends EntryBase {
  kind: "submenu";
  label: string;
  disabled?: boolean;
  items: MenuEntry[];
  /** Called each time the submenu opens (refresh a list). */
  onopen?: () => void;
  /** `data-testid` of the submenu popup. */
  menuTestid?: string;
  /** Minimum submenu width, px. */
  minWidth?: number;
}

export interface MenuSeparatorEntry {
  kind: "separator";
  id: string;
}

/** A group title ("Peak", "Loudness", "EQ" …): not focusable, names the items below it. */
export interface MenuHeadingEntry {
  kind: "heading";
  id: string;
  label: string;
}

/** Quiet non-interactive text ("No saved presets", "…" while loading). */
export interface MenuNoteEntry {
  kind: "note";
  id: string;
  label: string;
}

/** Inline content that isn't a menu item (a name field and its buttons). Keys typed inside it
 * never drive the menu, except Escape. */
export interface MenuCustomEntry {
  kind: "custom";
  id: string;
  content: Snippet;
}

export type MenuEntry =
  | MenuItemEntry
  | MenuCheckboxEntry
  | MenuRadioEntry
  | MenuSubmenuEntry
  | MenuSeparatorEntry
  | MenuHeadingEntry
  | MenuNoteEntry
  | MenuCustomEntry;

export type MenuActionEntry = MenuItemEntry | MenuCheckboxEntry | MenuRadioEntry | MenuSubmenuEntry;

export function isNavigable(entry: MenuEntry): entry is MenuActionEntry {
  return entry.kind === "item" || entry.kind === "checkbox" || entry.kind === "radio" || entry.kind === "submenu";
}

/** Why a menu closed — decides where focus goes next. */
export type MenuCloseReason = "select" | "escape" | "outside" | "tab";

/**
 * Next focus index for ArrowUp/ArrowDown/Home/End among `disabled.length` items, skipping
 * disabled ones and wrapping. `current` is -1 when nothing in the menu has focus yet (ArrowDown
 * then lands on the first item and ArrowUp on the last). `null` for other keys or when every
 * item is disabled.
 */
export function moveMenuFocus(current: number, key: string, disabled: readonly boolean[]): number | null {
  const count = disabled.length;
  if (count === 0 || disabled.every(Boolean)) {
    return null;
  }
  const firstFrom = (start: number, step: 1 | -1): number => {
    let index = start;
    for (let n = 0; n < count; n++) {
      index = (index + count) % count;
      if (!disabled[index]) {
        return index;
      }
      index += step;
    }
    return start;
  };
  switch (key) {
    case "Home":
      return firstFrom(0, 1);
    case "End":
      return firstFrom(count - 1, -1);
    case "ArrowDown":
      return firstFrom(current < 0 ? 0 : current + 1, 1);
    case "ArrowUp":
      return firstFrom(current < 0 ? count - 1 : current - 1, -1);
    default:
      return null;
  }
}

/**
 * Typeahead (APG): the next enabled item after `current` whose label starts with `query`
 * (case-insensitive), wrapping. Repeating one letter ("ss") cycles through items starting with
 * it. `null` when nothing matches.
 */
export function typeaheadIndex(
  labels: readonly string[],
  disabled: readonly boolean[],
  query: string,
  current: number,
): number | null {
  const q = query.toLocaleLowerCase();
  if (q === "") {
    return null;
  }
  const repeated = [...q].every((ch) => ch === q[0]);
  const count = labels.length;
  // A fresh multi-letter query re-matches the current item (typing "sa" after "s" stays on
  // "Save"); a single or repeated letter moves on to the next match.
  const startOffset = q.length > 1 && !repeated ? 0 : 1;
  for (let n = 0; n < count; n++) {
    const index = (current + startOffset + n + count) % count;
    if (disabled[index]) {
      continue;
    }
    const label = labels[index]!.toLocaleLowerCase();
    if (label.startsWith(q) || (repeated && label.startsWith(q[0]!))) {
      return index;
    }
  }
  return null;
}

/** A printable single character (typeahead input), not a shortcut chord. */
export function isTypeaheadKey(event: Pick<KeyboardEvent, "key" | "ctrlKey" | "metaKey" | "altKey">): boolean {
  return event.key.length === 1 && event.key !== " " && !event.ctrlKey && !event.metaKey && !event.altKey;
}

export const TYPEAHEAD_RESET_MS = 500;

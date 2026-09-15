import type { ActionId } from "./actions";
import { isPlatformMac, SHORTCUTS, type KeyBinding, type ShortcutDef } from "./registry";

/**
 * H-19: the menu bar's "shortcut shown next to each item" requirement — a small pure formatter on
 * top of the shortcut registry (the single source of truth for bindings, T-104/CLAUDE.md: "Types
 * shared with Rust are generated, never hand-copied" doesn't apply here, but the same spirit does
 * for shortcuts — never hand-type a shortcut string in a menu component). Platform-aware like
 * `matchBinding` itself (⌘ on macOS, Ctrl elsewhere).
 */

const CODE_LABELS: Record<string, string> = {
  Space: "Space",
  Home: "Home",
  End: "End",
  Delete: "Delete",
  Escape: "Esc",
  Equal: "=",
  Minus: "-",
  ArrowUp: "↑",
  ArrowDown: "↓",
  ArrowLeft: "←",
  ArrowRight: "→",
};

/** `KeyboardEvent.code` → a short display label: the mapped table above, else the trailing letter
 * of `KeyX`/digit of `DigitN`, else the raw code as a last resort (never seen today — every
 * binding in {@link SHORTCUTS} is covered by one of the first two cases). */
function keyLabel(code: string): string {
  const mapped = CODE_LABELS[code];
  if (mapped) {
    return mapped;
  }
  const key = /^Key([A-Z])$/.exec(code);
  if (key) {
    return key[1]!;
  }
  const digit = /^Digit(\d)$/.exec(code);
  if (digit) {
    return digit[1]!;
  }
  return code;
}

/** Formats one binding for display: mac uses the conventional symbol order with no separators
 * (⇧⌘Z), everyone else joins named modifiers with `+` (Ctrl+Shift+Z). */
export function formatBinding(binding: KeyBinding, isMac: boolean): string {
  if (isMac) {
    let label = "";
    if (binding.shift) {
      label += "⇧";
    }
    if (binding.alt) {
      label += "⌥";
    }
    if (binding.mod) {
      label += "⌘";
    }
    return label + keyLabel(binding.code);
  }
  const parts: string[] = [];
  if (binding.mod) {
    parts.push("Ctrl");
  }
  if (binding.shift) {
    parts.push("Shift");
  }
  if (binding.alt) {
    parts.push("Alt");
  }
  parts.push(keyLabel(binding.code));
  return parts.join("+");
}

/**
 * The display label for `action`'s default binding, or `undefined` if it has none (e.g. Silence,
 * the normalize favorites — "menu only" per `registry.ts`). Every action has at most one default
 * binding today (`findDuplicateBindings` would catch two bindings sharing a (code, shift, mod,
 * alt) identity, but nothing stops two different identities from targeting the same action — this
 * picks the first, which is exactly one for every action currently in {@link SHORTCUTS}).
 */
export function shortcutLabelForAction(
  action: ActionId,
  isMac: boolean = isPlatformMac(),
  bindings: readonly KeyBinding[] = SHORTCUTS,
): string | undefined {
  const binding = bindings.find((b) => b.action === action);
  return binding ? formatBinding(binding, isMac) : undefined;
}

/** T-701: one row for the Help ▸ Keyboard Shortcuts dialog (and reused by its test) — every
 * registry entry with its platform-formatted display label attached, in registry order. */
export interface ShortcutRow {
  readonly entry: ShortcutDef;
  readonly display: string;
}

export function shortcutRows(
  isMac: boolean = isPlatformMac(),
  entries: readonly ShortcutDef[] = SHORTCUTS,
): ShortcutRow[] {
  return entries.map((entry) => ({ entry, display: formatBinding(entry, isMac) }));
}

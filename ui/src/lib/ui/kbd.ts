/**
 * Splits a display shortcut from `keymap/shortcutLabel.ts` (`Ctrl+Shift+Z` or macOS `⇧⌘Z`)
 * into one chip per key for `Kbd`. Pure.
 */
const MAC_MODIFIERS = new Set(["⇧", "⌥", "⌘", "⌃"]);

export function splitShortcut(label: string): string[] {
  if (label === "") {
    return [];
  }
  if (label.length > 1 && label.includes("+")) {
    const parts = label.split("+");
    // "Ctrl++" (the plus key itself) → the last empty pieces collapse into "+".
    const keys = parts.filter((p) => p !== "");
    if (label.endsWith("++")) {
      keys.push("+");
    }
    return keys;
  }
  const chars = [...label];
  const keys: string[] = [];
  let i = 0;
  while (i < chars.length && MAC_MODIFIERS.has(chars[i]!)) {
    keys.push(chars[i]!);
    i++;
  }
  if (i < chars.length) {
    keys.push(chars.slice(i).join(""));
  }
  return keys;
}

export function usesSeparators(label: string): boolean {
  return label.length > 1 && label.includes("+");
}

const ARIA_NAMES: Record<string, string> = {
  Ctrl: "Control",
  "⌘": "Meta",
  "⇧": "Shift",
  "⌥": "Alt",
  "⌃": "Control",
  Esc: "Escape",
  "↑": "ArrowUp",
  "↓": "ArrowDown",
  "←": "ArrowLeft",
  "→": "ArrowRight",
};

/** Display label (`Ctrl+Shift+Z`, `⇧⌘Z`) → `aria-keyshortcuts` value (`Control+Shift+Z`). */
export function toAriaKeyShortcuts(label: string): string {
  return splitShortcut(label)
    .map((key) => ARIA_NAMES[key] ?? key)
    .join("+");
}

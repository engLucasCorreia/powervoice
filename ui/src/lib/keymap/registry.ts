import type { ActionId } from "./actions";
import { DEFAULT_KEYMAP, type KeyBinding } from "./bindings";

/** The subset of `KeyboardEvent` that matching needs — kept minimal so tests can pass a plain
 * object instead of constructing a real `KeyboardEvent`. */
export interface KeyEventLike {
  code: string;
  shiftKey: boolean;
  ctrlKey: boolean;
  metaKey: boolean;
  altKey: boolean;
}

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
 * macOS) doesn't accidentally satisfy a binding.
 */
export function matchBinding(event: KeyEventLike, isMac: boolean): ActionId | null {
  const modPressed = isMac ? event.metaKey : event.ctrlKey;
  const otherModPressed = isMac ? event.ctrlKey : event.metaKey;

  if (otherModPressed || event.altKey) {
    return null;
  }

  for (const binding of DEFAULT_KEYMAP) {
    if (
      binding.code === event.code &&
      Boolean(binding.shift) === event.shiftKey &&
      Boolean(binding.mod) === modPressed
    ) {
      return binding.action;
    }
  }
  return null;
}

/** A binding's unique key: (code, shift, mod). Two default bindings must never share one. */
function bindingIdentity(binding: KeyBinding): string {
  return `${binding.code}|shift=${Boolean(binding.shift)}|mod=${Boolean(binding.mod)}`;
}

/** Every duplicated (code, shift, mod) triple in the default keymap. Empty when bindings are unique. */
export function findDuplicateBindings(bindings: readonly KeyBinding[] = DEFAULT_KEYMAP): string[] {
  const seen = new Map<string, number>();
  for (const binding of bindings) {
    const identity = bindingIdentity(binding);
    seen.set(identity, (seen.get(identity) ?? 0) + 1);
  }
  return [...seen.entries()].filter(([, count]) => count > 1).map(([identity]) => identity);
}

export type { KeyBinding };
export { DEFAULT_KEYMAP };

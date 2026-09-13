import type { ActionId } from "./actions";
import { isPlatformMac, matchBinding } from "./registry";

type ActionHandler = () => void;

const handlers = new Map<ActionId, ActionHandler>();

/**
 * Registers the handler for `action`, replacing any previous one. Returns an unregister function.
 * Actions are dispatched to handlers registered by features later (ticket) — this module never
 * hard-codes what an action *does*, only which key resolves to which action id.
 */
export function registerAction(action: ActionId, handler: ActionHandler): () => void {
  handlers.set(action, handler);
  return () => {
    if (handlers.get(action) === handler) {
      handlers.delete(action);
    }
  };
}

/** Removes every registered handler (test/teardown helper). */
export function clearActionHandlers(): void {
  handlers.clear();
}

/** Dispatches `action` to its registered handler, or does nothing if none is registered
 * ("unknown/unhandled actions are no-ops", ticket). */
export function dispatchAction(action: ActionId): void {
  handlers.get(action)?.();
}

/**
 * True when `target` is a text input, textarea, or any `contenteditable` element — the ticket
 * requires ignoring key events while one of these has focus, so e.g. typing "z" while renaming a
 * marker never triggers undo.
 */
export function isEditableTarget(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) {
    return false;
  }
  // `isContentEditable` is the spec-correct check, but jsdom (used by Vitest) doesn't compute it
  // from the `contenteditable` attribute (known jsdom limitation), so also check the attribute
  // directly — real browsers agree with `isContentEditable` in that case too.
  const contentEditableAttr = target.getAttribute("contenteditable");
  if (target.isContentEditable || contentEditableAttr === "true" || contentEditableAttr === "") {
    return true;
  }
  const tag = target.tagName;
  if (tag === "TEXTAREA") {
    return true;
  }
  if (tag === "INPUT") {
    // Non-text input types (checkbox, range, button, ...) don't accept typed text, so the
    // keymap should still fire for them (e.g. Space toggling a checkbox is the browser's own
    // behavior, but Ctrl+Z on a `range` input shouldn't be swallowed as "editing text").
    const type = (target as HTMLInputElement).type;
    const nonTextTypes = new Set([
      "button",
      "checkbox",
      "color",
      "file",
      "image",
      "radio",
      "range",
      "reset",
      "submit",
    ]);
    return !nonTextTypes.has(type);
  }
  return false;
}

export interface AttachKeymapOptions {
  /** Overrides mac detection (tests only; production always uses the real platform). */
  isMac?: boolean;
}

/**
 * Attaches the keydown listener to `target` (defaults to `window`). Ignores events while an
 * editable element has focus, resolves the rest against the default keymap, and dispatches the
 * matched action. Returns a cleanup function that removes the listener.
 */
export function attachKeymap(
  target: Window = window,
  options: AttachKeymapOptions = {}
): () => void {
  const isMac = options.isMac ?? isPlatformMac();
  const onKeyDown = (event: KeyboardEvent) => {
    if (isEditableTarget(event.target)) {
      return;
    }
    const action = matchBinding(event, isMac);
    if (action) {
      event.preventDefault();
      dispatchAction(action);
    }
  };
  target.addEventListener("keydown", onKeyDown);
  return () => target.removeEventListener("keydown", onKeyDown);
}

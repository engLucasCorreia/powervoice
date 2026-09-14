/**
 * Shared keyboard navigation for one menu popup's own item list (H-19, WAI-ARIA menu pattern):
 * `ArrowUp`/`ArrowDown` move focus among the popup's direct-child `menuitem`/`menuitemcheckbox`/
 * `menuitemradio` elements (wrapping), `Home`/`End` jump to the first/last, disabled items are
 * skipped. `ArrowLeft`/`ArrowRight`/`Escape` are delegated to the caller via `options`, since they
 * mean different things for a top-level menu (switch to the sibling menu) versus a submenu popup
 * (close back to the parent item) — each menu component wires the right callbacks.
 *
 * Queries `:scope > [role^="menuitem"]` (direct children only) so a nested submenu's own items
 * (inside its own `role="menu"` popup, itself a *direct* child of this one) are never mixed into
 * this popup's own list — `role="menu"` doesn't match the `menuitem` prefix, and the nested
 * items are grandchildren, not direct children, of this popup.
 */

const ITEM_SELECTOR = ':scope > [role^="menuitem"]:not(:disabled)';

function ownItems(popupEl: HTMLElement): HTMLElement[] {
  return [...popupEl.querySelectorAll<HTMLElement>(ITEM_SELECTOR)];
}

export function focusFirstItem(popupEl: HTMLElement | null | undefined): void {
  if (!popupEl) {
    return;
  }
  ownItems(popupEl)[0]?.focus();
}

export function focusLastItem(popupEl: HTMLElement | null | undefined): void {
  if (!popupEl) {
    return;
  }
  ownItems(popupEl).at(-1)?.focus();
}

export interface MenuKeyboardOptions {
  /** `Escape`: close this popup (and typically refocus its own trigger). */
  onEscape?: () => void;
  /** `ArrowLeft` when this popup is a *submenu*: close back to the parent item. Takes priority
   * over `onArrowLeft` when both are given. */
  onCloseSubmenu?: () => void;
  /** `ArrowLeft` for a top-level menu: move to the previous sibling menu. */
  onArrowLeft?: () => void;
  /** `ArrowRight` for a top-level menu: move to the next sibling menu. Not called when the
   * currently focused item is itself a submenu trigger — the caller's own item `onclick`/open
   * logic handles opening it (this module doesn't know which items have submenus). */
  onArrowRight?: () => void;
}

/**
 * Handles one keydown on `popupEl` (the `role="menu"` container itself, or any of its own direct
 * items via bubbling). Stops propagation on every key it handles so an ancestor popup (a parent
 * menu, for a submenu) never double-handles the same keypress.
 */
export function handleMenuKeydown(
  popupEl: HTMLElement,
  event: KeyboardEvent,
  options: MenuKeyboardOptions = {},
): void {
  const items = ownItems(popupEl);
  const current = document.activeElement;
  const index = items.indexOf(current as HTMLElement);

  switch (event.key) {
    case "ArrowDown": {
      event.preventDefault();
      event.stopPropagation();
      items[(index + 1 + items.length) % items.length]?.focus();
      break;
    }
    case "ArrowUp": {
      event.preventDefault();
      event.stopPropagation();
      items[(index - 1 + items.length) % items.length]?.focus();
      break;
    }
    case "Home":
      event.preventDefault();
      event.stopPropagation();
      items[0]?.focus();
      break;
    case "End":
      event.preventDefault();
      event.stopPropagation();
      items.at(-1)?.focus();
      break;
    case "Escape":
      event.preventDefault();
      event.stopPropagation();
      options.onEscape?.();
      break;
    case "ArrowLeft":
      if (options.onCloseSubmenu) {
        event.preventDefault();
        event.stopPropagation();
        options.onCloseSubmenu();
      } else if (options.onArrowLeft) {
        event.preventDefault();
        event.stopPropagation();
        options.onArrowLeft();
      }
      break;
    case "ArrowRight":
      if (options.onArrowRight) {
        event.preventDefault();
        event.stopPropagation();
        options.onArrowRight();
      }
      break;
    default:
      break;
  }
}

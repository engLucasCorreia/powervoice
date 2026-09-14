import { describe, expect, it, vi } from "vitest";
import { focusFirstItem, focusLastItem, handleMenuKeydown } from "./menuKeyboard";

function buildPopup(): { popup: HTMLDivElement; items: HTMLButtonElement[] } {
  const popup = document.createElement("div");
  popup.setAttribute("role", "menu");
  document.body.appendChild(popup);

  const items: HTMLButtonElement[] = [];
  for (const [label, disabled] of [["A", false], ["B", true], ["C", false]] as const) {
    const button = document.createElement("button");
    button.setAttribute("role", "menuitem");
    button.textContent = label;
    button.disabled = disabled;
    popup.appendChild(button);
    items.push(button);
  }
  return { popup, items };
}

function keydown(key: string): KeyboardEvent {
  return new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true });
}

describe("menuKeyboard (H-19)", () => {
  it("focusFirstItem/focusLastItem skip disabled items", () => {
    const { popup, items } = buildPopup();
    focusFirstItem(popup);
    expect(document.activeElement).toBe(items[0]);
    focusLastItem(popup);
    expect(document.activeElement).toBe(items[2]);
  });

  it("ArrowDown/ArrowUp move focus among enabled items, wrapping and skipping disabled ones", () => {
    const { popup, items } = buildPopup();
    items[0]!.focus();

    handleMenuKeydown(popup, keydown("ArrowDown"));
    expect(document.activeElement).toBe(items[2]); // B is disabled, skipped

    handleMenuKeydown(popup, keydown("ArrowDown"));
    expect(document.activeElement).toBe(items[0]); // wraps

    handleMenuKeydown(popup, keydown("ArrowUp"));
    expect(document.activeElement).toBe(items[2]); // wraps backward, skipping B
  });

  it("Home/End jump to the first/last enabled item", () => {
    const { popup, items } = buildPopup();
    items[2]!.focus();
    handleMenuKeydown(popup, keydown("Home"));
    expect(document.activeElement).toBe(items[0]);
    handleMenuKeydown(popup, keydown("End"));
    expect(document.activeElement).toBe(items[2]);
  });

  it("Escape calls onEscape", () => {
    const { popup } = buildPopup();
    const onEscape = vi.fn();
    handleMenuKeydown(popup, keydown("Escape"), { onEscape });
    expect(onEscape).toHaveBeenCalledOnce();
  });

  it("ArrowLeft prefers onCloseSubmenu over onArrowLeft when both are given", () => {
    const { popup } = buildPopup();
    const onCloseSubmenu = vi.fn();
    const onArrowLeft = vi.fn();
    handleMenuKeydown(popup, keydown("ArrowLeft"), { onCloseSubmenu, onArrowLeft });
    expect(onCloseSubmenu).toHaveBeenCalledOnce();
    expect(onArrowLeft).not.toHaveBeenCalled();
  });

  it("ArrowLeft falls back to onArrowLeft for a top-level menu (no onCloseSubmenu)", () => {
    const { popup } = buildPopup();
    const onArrowLeft = vi.fn();
    handleMenuKeydown(popup, keydown("ArrowLeft"), { onArrowLeft });
    expect(onArrowLeft).toHaveBeenCalledOnce();
  });

  it("ArrowRight calls onArrowRight when given", () => {
    const { popup } = buildPopup();
    const onArrowRight = vi.fn();
    handleMenuKeydown(popup, keydown("ArrowRight"), { onArrowRight });
    expect(onArrowRight).toHaveBeenCalledOnce();
  });

  it("only queries this popup's own direct-child items, not a nested submenu's", () => {
    const { popup, items } = buildPopup();
    const nested = document.createElement("div");
    nested.setAttribute("role", "menu");
    const nestedItem = document.createElement("button");
    nestedItem.setAttribute("role", "menuitem");
    nestedItem.textContent = "nested";
    nested.appendChild(nestedItem);
    popup.appendChild(nested);

    items[0]!.focus();
    handleMenuKeydown(popup, keydown("End"));
    // The last item of the OUTER popup is still "C" (items[2]), not the nested item — a
    // `role="menu"` wrapper doesn't match the `menuitem` prefix, and the nested item is a
    // grandchild, not a direct child, of the outer popup.
    expect(document.activeElement).toBe(items[2]);
  });

  it("stops propagation on a handled key so an ancestor popup doesn't also react", () => {
    const { popup, items } = buildPopup();
    items[0]!.focus();
    const event = keydown("ArrowDown");
    handleMenuKeydown(popup, event);
    expect(event.cancelBubble).toBe(true);
  });

  it("does not stop propagation for an unhandled key", () => {
    const { popup } = buildPopup();
    const event = keydown("Tab");
    handleMenuKeydown(popup, event);
    expect(event.cancelBubble).toBe(false);
  });
});

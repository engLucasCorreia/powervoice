import { afterEach, beforeEach, describe, expect, it } from "vitest";
import {
  MENU_ORDER,
  attachMenuBarMnemonics,
  closeAllMenus,
  focusMenuTrigger,
  menubarState,
  moveToAdjacentMenu,
  openMenu,
  resetMenuBarForTest,
  toggleMenu,
} from "./menu.svelte";

function mountTriggers(): HTMLElement[] {
  const container = document.createElement("div");
  document.body.appendChild(container);
  return MENU_ORDER.map((id) => {
    const button = document.createElement("button");
    button.setAttribute("data-menu-trigger", id);
    container.appendChild(button);
    return button;
  });
}

afterEach(() => {
  resetMenuBarForTest();
  document.body.innerHTML = "";
});

describe("menubar store (H-19)", () => {
  it("opens, toggles and closes by id", () => {
    expect(menubarState().openMenuId).toBeNull();
    openMenu("edit");
    expect(menubarState().openMenuId).toBe("edit");
    toggleMenu("edit");
    expect(menubarState().openMenuId).toBeNull();
    toggleMenu("view");
    expect(menubarState().openMenuId).toBe("view");
    // Opening a different menu id (or toggling another one on) replaces the open one — only one
    // menu is ever open at a time.
    toggleMenu("file");
    expect(menubarState().openMenuId).toBe("file");
    closeAllMenus();
    expect(menubarState().openMenuId).toBeNull();
  });

  it("focusMenuTrigger focuses the matching trigger element", () => {
    const [file] = mountTriggers();
    focusMenuTrigger("file");
    expect(document.activeElement).toBe(file);
  });

  it("moveToAdjacentMenu wraps around the menu order and always moves focus", () => {
    const triggers = mountTriggers();
    moveToAdjacentMenu("help", 1); // wraps past the end back to "file"
    expect(document.activeElement).toBe(triggers[0]);
    moveToAdjacentMenu("file", -1); // wraps back to "help"
    expect(document.activeElement).toBe(triggers[triggers.length - 1]);
  });

  it("moveToAdjacentMenu only opens the adjacent menu if one was already open", () => {
    mountTriggers();
    // Nothing open: focus moves, but nothing opens.
    moveToAdjacentMenu("file", 1);
    expect(menubarState().openMenuId).toBeNull();

    // A menu open: the adjacent one opens too (cascading through the bar).
    openMenu("file");
    moveToAdjacentMenu("file", 1);
    expect(menubarState().openMenuId).toBe("edit");
  });

  describe("attachMenuBarMnemonics", () => {
    let detach: () => void;

    beforeEach(() => {
      detach = attachMenuBarMnemonics(window);
    });

    afterEach(() => {
      detach();
    });

    it("Alt+letter opens the matching menu and focuses its trigger", () => {
      const triggers = mountTriggers();
      const event = new KeyboardEvent("keydown", { key: "v", altKey: true, bubbles: true });
      window.dispatchEvent(event);
      expect(menubarState().openMenuId).toBe("view");
      expect(document.activeElement).toBe(triggers[MENU_ORDER.indexOf("view")]);
    });

    it("ignores Alt+letter combined with Ctrl or Cmd", () => {
      mountTriggers();
      window.dispatchEvent(
        new KeyboardEvent("keydown", { key: "v", altKey: true, ctrlKey: true, bubbles: true }),
      );
      expect(menubarState().openMenuId).toBeNull();
    });

    it("does nothing for a letter with no matching mnemonic", () => {
      mountTriggers();
      window.dispatchEvent(new KeyboardEvent("keydown", { key: "z", altKey: true, bubbles: true }));
      expect(menubarState().openMenuId).toBeNull();
    });
  });
});

import { createRawSnippet, flushSync, tick } from "svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import Menu from "./Menu.svelte";
import type { MenuCloseReason, MenuEntry } from "./menuModel";
import { byTestId, click, key, render, type Rendered } from "./testing";

let r: Rendered | null = null;
let anchor: HTMLButtonElement;

beforeEach(() => {
  anchor = document.createElement("button");
  anchor.textContent = "Trigger";
  document.body.appendChild(anchor);
});

afterEach(() => {
  r?.cleanup();
  r = null;
  document.body.innerHTML = "";
});

function entries(overrides: { onSave?: () => void; onNested?: () => void } = {}): MenuEntry[] {
  return [
    { kind: "item", id: "new", label: "New", testid: "new", shortcut: "Ctrl+N", onselect: vi.fn() },
    { kind: "item", id: "open", label: "Open…", testid: "open", disabled: true, onselect: vi.fn() },
    { kind: "separator", id: "sep" },
    { kind: "heading", id: "view", label: "View" },
    { kind: "checkbox", id: "spectral", label: "Spectral", testid: "spectral", checked: true, onselect: vi.fn() },
    { kind: "radio", id: "auto", label: "Automatic", testid: "auto", checked: false, onselect: vi.fn() },
    {
      kind: "submenu",
      id: "more",
      label: "More",
      testid: "more",
      menuTestid: "more-menu",
      items: [
        { kind: "item", id: "a", label: "Alpha", testid: "alpha", onselect: overrides.onNested ?? vi.fn() },
        { kind: "item", id: "b", label: "Beta", testid: "beta", onselect: vi.fn() },
      ],
    },
    { kind: "item", id: "save", label: "Save", testid: "save", onselect: overrides.onSave ?? vi.fn() },
  ];
}

function mountMenu(
  items: MenuEntry[],
  onclose: (reason: MenuCloseReason) => void = vi.fn(),
  extra: Record<string, unknown> = {},
): HTMLElement {
  r = render(Menu, { open: true, anchor, items, label: "File", testid: "menu", onclose, ...extra });
  return byTestId(r.target, "menu");
}

const focused = (): string | null => (document.activeElement as HTMLElement | null)?.dataset.testid ?? null;

describe("Menu (H-26 shared menu)", () => {
  it("is a labelled role=menu with menuitem / menuitemcheckbox / menuitemradio rows", () => {
    const menu = mountMenu(entries());
    expect(menu.getAttribute("role")).toBe("menu");
    expect(menu.getAttribute("aria-label")).toBe("File");
    expect(byTestId(menu, "new").getAttribute("role")).toBe("menuitem");
    expect(byTestId(menu, "spectral").getAttribute("role")).toBe("menuitemcheckbox");
    expect(byTestId(menu, "spectral").getAttribute("aria-checked")).toBe("true");
    expect(byTestId(menu, "auto").getAttribute("role")).toBe("menuitemradio");
    expect(byTestId(menu, "auto").getAttribute("aria-checked")).toBe("false");
    expect(byTestId(menu, "more").getAttribute("aria-haspopup")).toBe("menu");
    expect(byTestId(menu, "more").getAttribute("aria-expanded")).toBe("false");
    expect(byTestId<HTMLButtonElement>(menu, "open").disabled).toBe(true);
    expect(menu.querySelectorAll('[role="separator"]').length).toBe(1);
    // Shortcuts are right-aligned Kbd chips; the text still reads as the label.
    expect(menu.querySelector('[data-testid="new"] .shortcut kbd.pv-kbd')).not.toBeNull();
    expect(menu.querySelector('[data-testid="new"] .shortcut')?.textContent).toBe("Ctrl+N");
    expect(byTestId(menu, "new").getAttribute("aria-keyshortcuts")).toBe("Control+N");
  });

  it("focuses the first enabled item on open; arrows skip disabled rows and wrap; Home/End jump", () => {
    const menu = mountMenu(entries());
    expect(focused()).toBe("new");
    key(menu, "ArrowDown");
    expect(focused()).toBe("spectral"); // "Open…" is disabled
    key(menu, "ArrowUp");
    expect(focused()).toBe("new");
    key(menu, "ArrowUp");
    expect(focused()).toBe("save"); // wraps
    key(menu, "Home");
    expect(focused()).toBe("new");
    key(menu, "End");
    expect(focused()).toBe("save");
  });

  it("typeahead moves to the next item starting with the typed letter", () => {
    const menu = mountMenu(entries());
    key(menu, "s");
    expect(focused()).toBe("spectral");
    key(menu, "s");
    expect(focused()).toBe("save");
    key(menu, "a");
    // "sa…" is still the typed buffer within 500 ms → stays on Save.
    expect(focused()).toBe("save");
  });

  it("Enter and Space activate the focused item and close with reason 'select'", () => {
    const onSave = vi.fn();
    const onclose = vi.fn();
    const menu = mountMenu(entries({ onSave }), onclose);
    key(menu, "End");
    key(menu, "Enter");
    expect(onSave).toHaveBeenCalledOnce();
    expect(onclose).toHaveBeenCalledWith("select");
    key(menu, "End");
    key(menu, " ");
    expect(onSave).toHaveBeenCalledTimes(2);
  });

  it("Escape closes and returns focus to the trigger", () => {
    const onclose = vi.fn();
    const menu = mountMenu(entries(), onclose);
    expect(document.activeElement).not.toBe(anchor);
    key(menu, "Escape");
    expect(onclose).toHaveBeenCalledWith("escape");
    expect(document.activeElement).toBe(anchor);
  });

  it("selecting with the pointer also gives focus back to the trigger before the action runs", () => {
    let focusDuringAction: Element | null = null;
    const menu = mountMenu(entries({ onSave: () => (focusDuringAction = document.activeElement) }));
    click(byTestId(menu, "save"));
    expect(focusDuringAction).toBe(anchor);
  });

  it("Tab closes the whole menu", () => {
    const onclose = vi.fn();
    const menu = mountMenu(entries(), onclose);
    key(menu, "Tab");
    expect(onclose).toHaveBeenCalledWith("tab");
  });

  it("a pointer press outside closes it; inside or on the trigger does not", () => {
    const onclose = vi.fn();
    const menu = mountMenu(entries(), onclose);
    menu.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true }));
    anchor.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true }));
    expect(onclose).not.toHaveBeenCalled();
    document.body.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true }));
    expect(onclose).toHaveBeenCalledWith("outside");
  });

  it("→ opens a submenu on its first item, ← closes it back to its row", async () => {
    const onclose = vi.fn();
    const menu = mountMenu(entries(), onclose);
    key(menu, "End");
    key(menu, "ArrowUp");
    expect(focused()).toBe("more");
    key(menu, "ArrowRight");
    const sub = byTestId(r!.target, "more-menu");
    expect(sub.getAttribute("role")).toBe("menu");
    expect(byTestId(menu, "more").getAttribute("aria-expanded")).toBe("true");
    expect(focused()).toBe("alpha");
    key(sub, "ArrowDown");
    expect(focused()).toBe("beta");
    key(sub, "ArrowLeft");
    await tick();
    flushSync();
    expect(r!.target.querySelector('[data-testid="more-menu"]')).toBeNull();
    expect(focused()).toBe("more");
    expect(onclose).not.toHaveBeenCalled();
  });

  it("Escape in a submenu closes only the submenu; choosing in it closes everything", async () => {
    const onNested = vi.fn();
    const onclose = vi.fn();
    const menu = mountMenu(entries({ onNested }), onclose);
    click(byTestId(menu, "more"));
    let sub = byTestId(r!.target, "more-menu");
    key(sub, "Escape");
    await tick();
    expect(onclose).not.toHaveBeenCalled();
    expect(r!.target.querySelector('[data-testid="more-menu"]')).toBeNull();

    click(byTestId(menu, "more"));
    sub = byTestId(r!.target, "more-menu");
    click(byTestId(sub, "alpha"));
    expect(onNested).toHaveBeenCalledOnce();
    expect(onclose).toHaveBeenCalledWith("select");
    expect(document.activeElement).toBe(anchor);
  });

  it("← / → with no submenu move between menu-bar menus", () => {
    const onnavigate = vi.fn();
    const menu = mountMenu(entries(), vi.fn(), { onnavigate });
    key(menu, "ArrowRight");
    key(menu, "ArrowLeft");
    expect(onnavigate.mock.calls).toEqual([[1], [-1]]);
  });

  it("keepOpen items run without closing; a trailing action runs by click or Delete", () => {
    const onStep = vi.fn();
    const onTrailing = vi.fn();
    const onclose = vi.fn();
    const menu = mountMenu(
      [
        { kind: "item", id: "step", label: "Save as…", testid: "step", keepOpen: true, onselect: onStep },
        {
          kind: "item",
          id: "preset",
          label: "Mine",
          testid: "preset",
          onselect: vi.fn(),
          trailing: { icon: "delete", label: "Delete", testid: "preset-delete", onselect: onTrailing },
        },
      ],
      onclose,
    );
    click(byTestId(menu, "step"));
    expect(onStep).toHaveBeenCalledOnce();
    click(byTestId(menu, "preset-delete"));
    key(menu, "ArrowDown");
    key(menu, "Delete");
    expect(onTrailing).toHaveBeenCalledTimes(2);
    expect(onclose).not.toHaveBeenCalled();
  });

  it("keys typed into inline content don't drive the menu", () => {
    const content = createRawSnippet(() => ({ render: () => `<input type="text" data-testid="name" />` }));
    const menu = mountMenu([
      { kind: "item", id: "x", label: "Xylophone", testid: "x", onselect: vi.fn() },
      { kind: "custom", id: "form", content },
    ]);
    const input = byTestId<HTMLInputElement>(menu, "name");
    input.focus();
    key(input, "x");
    key(input, "ArrowUp");
    expect(document.activeElement).toBe(input);
  });

  it("opens at a point for context menus", () => {
    r = render(Menu, {
      open: true,
      anchor: { x: 40, y: 60 },
      items: entries(),
      label: "Record",
      testid: "ctx",
      onclose: vi.fn(),
    });
    expect(byTestId(r.target, "ctx").dataset.placement).toBe("bottom-start");
  });
});

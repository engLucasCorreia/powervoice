import { afterEach, describe, expect, it } from "vitest";
import Kbd from "./Kbd.svelte";
import { splitShortcut, toAriaKeyShortcuts } from "./kbd";
import { render, type Rendered } from "./testing";

let r: Rendered | null = null;
afterEach(() => {
  r?.cleanup();
  r = null;
});

describe("splitShortcut", () => {
  it("splits Ctrl/Shift/Alt labels on +", () => {
    expect(splitShortcut("Ctrl+Shift+Z")).toEqual(["Ctrl", "Shift", "Z"]);
    expect(splitShortcut("Shift+Space")).toEqual(["Shift", "Space"]);
    expect(splitShortcut("Ctrl+-")).toEqual(["Ctrl", "-"]);
    expect(splitShortcut("Ctrl+=")).toEqual(["Ctrl", "="]);
  });

  it("splits macOS symbol labels into one chip per modifier", () => {
    expect(splitShortcut("⇧⌘Z")).toEqual(["⇧", "⌘", "Z"]);
    expect(splitShortcut("⌘0")).toEqual(["⌘", "0"]);
  });

  it("keeps single keys whole", () => {
    expect(splitShortcut("Space")).toEqual(["Space"]);
    expect(splitShortcut("M")).toEqual(["M"]);
    expect(splitShortcut("")).toEqual([]);
  });
});

describe("Kbd", () => {
  it("renders a key combination as nested <kbd> elements", () => {
    r = render(Kbd, { keys: "Ctrl+Shift+Z" });
    const outer = r.target.querySelector("kbd.pv-kbd");
    expect(outer).not.toBeNull();
    const inner = outer?.querySelectorAll("kbd.key") ?? [];
    expect([...inner].map((k) => k.textContent)).toEqual(["Ctrl", "Shift", "Z"]);
    expect(outer?.textContent).toBe("Ctrl+Shift+Z");
  });

  it("macOS labels render without separators", () => {
    r = render(Kbd, { keys: "⇧⌘Z" });
    expect(r.target.querySelector("kbd.pv-kbd")?.textContent).toBe("⇧⌘Z");
  });
});

describe("toAriaKeyShortcuts (display label → aria-keyshortcuts)", () => {
  it("maps named modifiers and keys to the ARIA/UI Events spelling", () => {
    expect(toAriaKeyShortcuts("Ctrl+Shift+Z")).toBe("Control+Shift+Z");
    expect(toAriaKeyShortcuts("Shift+R")).toBe("Shift+R");
    expect(toAriaKeyShortcuts("Space")).toBe("Space");
    expect(toAriaKeyShortcuts("Ctrl+Alt+→")).toBe("Control+Alt+ArrowRight");
    expect(toAriaKeyShortcuts("Esc")).toBe("Escape");
  });

  it("maps macOS symbols", () => {
    expect(toAriaKeyShortcuts("⇧⌘Z")).toBe("Shift+Meta+Z");
  });
});

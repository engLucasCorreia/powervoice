import { afterEach, describe, expect, it } from "vitest";
import Gallery from "./Gallery.svelte";
import { ICON_NAMES } from "./icons";
import { byTestId, click, render, type Rendered } from "./testing";
import { flushSync } from "svelte";

let r: Rendered | null = null;
afterEach(() => {
  r?.cleanup();
  r = null;
});

describe("Gallery (dev page)", () => {
  it("renders the same showcase in a Dark, Light and High Contrast column (H-31)", () => {
    r = render(Gallery, {});
    const dark = byTestId(r.target, "gallery-dark");
    const light = byTestId(r.target, "gallery-light");
    const highContrast = byTestId(r.target, "gallery-high-contrast");
    expect(dark.dataset.theme).toBe("dark");
    expect(light.dataset.theme).toBe("light");
    expect(highContrast.dataset.theme).toBe("high-contrast");
    expect(dark.querySelectorAll("section.card").length).toBe(light.querySelectorAll("section.card").length);
    expect(dark.querySelectorAll("section.card").length).toBe(highContrast.querySelectorAll("section.card").length);
    expect(highContrast.querySelector(".column-title")?.textContent).toBe("High Contrast");
  });

  it("shows every component family in each theme", () => {
    r = render(Gallery, {});
    for (const theme of ["dark", "light", "high-contrast"]) {
      const col = byTestId(r.target, `gallery-${theme}`);
      expect(col.querySelectorAll(".pv-button").length, theme).toBeGreaterThanOrEqual(8);
      expect(col.querySelectorAll(".pv-icon-button").length).toBeGreaterThanOrEqual(8);
      expect(col.querySelectorAll('[role="radiogroup"]').length).toBeGreaterThanOrEqual(2);
      expect(col.querySelectorAll('[role="switch"]').length).toBeGreaterThanOrEqual(2);
      expect(col.querySelectorAll('[role="slider"]').length).toBeGreaterThanOrEqual(2);
      expect(col.querySelectorAll('[role="spinbutton"]').length).toBeGreaterThanOrEqual(2);
      expect(col.querySelectorAll("select").length).toBeGreaterThanOrEqual(2);
      expect(col.querySelectorAll('[role="tablist"]').length).toBeGreaterThanOrEqual(1);
      expect(col.querySelectorAll(".pv-panel-header").length).toBeGreaterThanOrEqual(2);
      expect(col.querySelectorAll(".pv-badge").length).toBeGreaterThanOrEqual(6);
      expect(col.querySelectorAll(".pv-status").length).toBeGreaterThanOrEqual(3);
      expect(col.querySelectorAll(".pv-readout").length).toBeGreaterThanOrEqual(3);
      expect(col.querySelectorAll("kbd.pv-kbd").length).toBeGreaterThanOrEqual(3);
      expect(col.querySelectorAll('[role="separator"]').length).toBeGreaterThanOrEqual(2);
      expect(col.querySelectorAll(".pv-empty").length).toBe(1);
      expect(col.querySelectorAll(".icon-grid svg").length).toBe(ICON_NAMES.length);
    }
  });

  it("the transport demo goes on air when Record is pressed", () => {
    r = render(Gallery, {});
    const bar = byTestId(r.target, "gallery-dark").querySelector<HTMLElement>(".transport-demo");
    expect(bar?.classList.contains("on-air")).toBe(false);
    const record = bar?.querySelector<HTMLButtonElement>('[data-variant="record"]');
    click(record!);
    expect(bar?.classList.contains("on-air")).toBe(true);
    expect(record?.dataset.active).toBe("true");
    expect(bar?.querySelector('.pv-status[data-tone="record"]')?.textContent?.trim()).toBe("Recording");
  });

  it("shows the shared menu, a popover and both dialog button orders (H-26)", () => {
    r = render(Gallery, {});
    const card = byTestId(r.target, "gallery-menus-dark");
    const trigger = card.querySelector<HTMLButtonElement>('[aria-haspopup="menu"]')!;
    click(trigger);
    flushSync();
    const menu = card.querySelector('[role="menu"]');
    expect(menu).not.toBeNull();
    expect(menu!.querySelectorAll('[role="menuitemcheckbox"]').length).toBe(1);
    const orders = [...card.querySelectorAll<HTMLElement>(".footer-demo")].map((demo) =>
      [...demo.querySelectorAll(".pv-button")].map((b) => b.textContent?.trim()),
    );
    expect(orders).toEqual([
      ["Don't Save", "Cancel", "Save"],
      ["Save", "Don't Save", "Cancel"],
    ]);
  });
});

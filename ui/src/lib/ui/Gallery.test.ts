import { afterEach, describe, expect, it } from "vitest";
import Gallery from "./Gallery.svelte";
import { ICON_NAMES } from "./icons";
import { byTestId, click, render, type Rendered } from "./testing";

let r: Rendered | null = null;
afterEach(() => {
  r?.cleanup();
  r = null;
});

describe("Gallery (dev page)", () => {
  it("renders the same showcase in a dark and a light column", () => {
    r = render(Gallery, {});
    const dark = byTestId(r.target, "gallery-dark");
    const light = byTestId(r.target, "gallery-light");
    expect(dark.dataset.theme).toBe("dark");
    expect(light.dataset.theme).toBe("light");
    expect(dark.querySelectorAll("section.card").length).toBe(light.querySelectorAll("section.card").length);
  });

  it("shows every component family in each theme", () => {
    r = render(Gallery, {});
    for (const theme of ["dark", "light"]) {
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
});

import { afterEach, describe, expect, it, vi } from "vitest";
import PanelHeader from "./PanelHeader.svelte";
import { click, render, textSnippet, type Rendered } from "./testing";

let r: Rendered | null = null;
afterEach(() => {
  r?.cleanup();
  r = null;
});

describe("PanelHeader", () => {
  it("renders the title as a heading (h2 by default) with meta and actions", () => {
    r = render(PanelHeader, { title: "Markers", meta: "3 markers", actions: textSnippet("ACTIONS") });
    const h = r.target.querySelector("h2");
    expect(h?.textContent?.trim()).toBe("Markers");
    expect(r.target.querySelector(".meta")?.textContent).toBe("3 markers");
    expect(r.target.querySelector(".actions")?.textContent).toBe("ACTIONS");
    expect(r.target.querySelector("button[aria-expanded]")).toBeNull();
  });

  it("supports h3", () => {
    r = render(PanelHeader, { title: "Punch", level: 3 });
    expect(r.target.querySelector("h3")?.textContent?.trim()).toBe("Punch");
  });

  it("collapsible: a disclosure button inside the heading toggles aria-expanded", () => {
    const ontoggle = vi.fn();
    r = render(PanelHeader, { title: "Rack", collapsible: true, controls: "rack-body", ontoggle });
    const button = r.target.querySelector<HTMLButtonElement>("h2 > button");
    expect(button?.getAttribute("aria-expanded")).toBe("true");
    expect(button?.getAttribute("aria-controls")).toBe("rack-body");
    expect(button?.textContent?.trim()).toBe("Rack");
    click(button!);
    expect(button?.getAttribute("aria-expanded")).toBe("false");
    expect(ontoggle).toHaveBeenCalledWith(false);
  });
});

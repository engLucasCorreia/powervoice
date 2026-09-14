import { createRawSnippet } from "svelte";
import { afterEach, describe, expect, it, vi } from "vitest";
import Tabs from "./Tabs.svelte";
import { key, click, render, type Rendered } from "./testing";

const TABS = [
  { id: "meters", label: "Meters" },
  { id: "analyzer", label: "Analyzer" },
  { id: "loudness", label: "Loudness", badge: "!" },
];

const panel = createRawSnippet<[string]>((id) => ({
  render: () => `<p data-testid="panel-content">${id()}</p>`,
}));

let r: Rendered | null = null;
afterEach(() => {
  r?.cleanup();
  r = null;
});

function tabs(root: HTMLElement): HTMLButtonElement[] {
  return [...root.querySelectorAll<HTMLButtonElement>('[role="tab"]')];
}

describe("Tabs", () => {
  it("wires tablist/tab/tabpanel with ids, aria-selected and roving tabindex", () => {
    r = render(Tabs, { tabs: TABS, selected: "analyzer", label: "Bottom dock", idPrefix: "dock", panel });
    const list = r.target.querySelector('[role="tablist"]');
    expect(list?.getAttribute("aria-label")).toBe("Bottom dock");
    const [meters, analyzer] = tabs(r.target);
    expect(analyzer?.getAttribute("aria-selected")).toBe("true");
    expect(meters?.getAttribute("aria-selected")).toBe("false");
    expect(analyzer?.tabIndex).toBe(0);
    expect(meters?.tabIndex).toBe(-1);
    const tabpanel = r.target.querySelector('[role="tabpanel"]');
    expect(analyzer?.getAttribute("aria-controls")).toBe(tabpanel?.id);
    expect(tabpanel?.getAttribute("aria-labelledby")).toBe(analyzer?.id);
    expect(r.target.querySelector('[data-testid="panel-content"]')?.textContent).toBe("analyzer");
  });

  it("click selects a tab and swaps the panel", () => {
    const onchange = vi.fn();
    r = render(Tabs, { tabs: TABS, selected: "meters", label: "Dock", idPrefix: "d", panel, onchange });
    click(tabs(r.target)[2]!);
    expect(onchange).toHaveBeenCalledWith("loudness");
    expect(tabs(r.target)[2]?.getAttribute("aria-selected")).toBe("true");
    const tabpanel = r.target.querySelector('[role="tabpanel"]');
    expect(tabpanel?.id).toBe("d-panel-loudness");
    expect(tabpanel?.getAttribute("aria-labelledby")).toBe("d-tab-loudness");
  });

  it("Left/Right/Home/End move focus and select (automatic activation)", () => {
    const onchange = vi.fn();
    r = render(Tabs, { tabs: TABS, selected: "meters", label: "Dock", idPrefix: "d", panel, onchange });
    key(tabs(r.target)[0]!, "ArrowRight");
    expect(onchange).toHaveBeenLastCalledWith("analyzer");
    expect(document.activeElement).toBe(tabs(r.target)[1]);
    key(tabs(r.target)[1]!, "End");
    expect(onchange).toHaveBeenLastCalledWith("loudness");
    key(tabs(r.target)[2]!, "Home");
    expect(onchange).toHaveBeenLastCalledWith("meters");
    key(tabs(r.target)[0]!, "ArrowDown");
    expect(onchange).toHaveBeenCalledTimes(3);
  });

  it("shows a badge next to a tab label", () => {
    r = render(Tabs, { tabs: TABS, selected: "meters", label: "Dock", idPrefix: "d" });
    expect(tabs(r.target)[2]?.querySelector(".badge")?.textContent).toBe("!");
    expect(r.target.querySelector('[role="tabpanel"]')).toBeNull();
  });
});

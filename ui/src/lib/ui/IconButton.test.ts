import { flushSync } from "svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import IconButton from "./IconButton.svelte";
import { resetTooltipsForTest } from "./tooltip";
import { byTestId, click, render, type Rendered } from "./testing";

let r: Rendered | null = null;
beforeEach(() => resetTooltipsForTest());
afterEach(() => {
  r?.cleanup();
  r = null;
  vi.useRealTimers();
});

describe("IconButton", () => {
  it("is named by its label and draws a decorative icon", () => {
    r = render(IconButton, { icon: "play", label: "Play", testid: "ib" });
    const b = byTestId<HTMLButtonElement>(r.target, "ib");
    expect(b.tagName).toBe("BUTTON");
    expect(b.type).toBe("button");
    expect(b.getAttribute("aria-label")).toBe("Play");
    expect(b.querySelector("svg")?.getAttribute("aria-hidden")).toBe("true");
    expect(b.getAttribute("aria-pressed")).toBeNull();
  });

  it("shows its label (and shortcut) as a tooltip, without duplicating the name as a description", () => {
    vi.useFakeTimers();
    r = render(IconButton, { icon: "record", label: "Record", shortcut: "Shift+R", testid: "ib" });
    const b = byTestId(r.target, "ib");
    const tip = r.target.querySelector<HTMLElement>('[role="tooltip"]');
    expect(tip?.textContent).toContain("Record");
    expect(tip?.querySelector("kbd")?.textContent).toBe("Shift+R");
    expect(b.getAttribute("aria-describedby")).toBeNull();
    expect(b.getAttribute("aria-keyshortcuts")).toBe("Shift+R");
    b.dispatchEvent(new FocusEvent("focusin", { bubbles: true }));
    flushSync();
    expect(tip?.hidden).toBe(false);
  });

  it("toggle icon buttons expose aria-pressed", () => {
    r = render(IconButton, { icon: "loop", label: "Loop", pressed: true, testid: "ib" });
    expect(byTestId(r.target, "ib").getAttribute("aria-pressed")).toBe("true");
  });

  it("clicks call onclick; disabled blocks it", () => {
    const onclick = vi.fn();
    r = render(IconButton, { icon: "stop", label: "Stop", onclick, testid: "ib" });
    click(byTestId(r.target, "ib"));
    expect(onclick).toHaveBeenCalledTimes(1);
    r.cleanup();
    const blocked = vi.fn();
    r = render(IconButton, { icon: "stop", label: "Stop", onclick: blocked, disabled: true, testid: "ib" });
    click(byTestId(r.target, "ib"));
    expect(blocked).not.toHaveBeenCalled();
  });

  it("tooltip={false} renders the bare button", () => {
    r = render(IconButton, { icon: "close", label: "Close", tooltip: false, testid: "ib" });
    expect(r.target.querySelector('[role="tooltip"]')).toBeNull();
    expect(byTestId(r.target, "ib").getAttribute("aria-label")).toBe("Close");
  });

  it("exposes variant/size/active hooks (the record button's on-air state)", () => {
    r = render(IconButton, {
      icon: "record",
      label: "Stop recording",
      variant: "record",
      size: "lg",
      active: true,
      testid: "ib",
    });
    const b = byTestId(r.target, "ib");
    expect(b.dataset.variant).toBe("record");
    expect(b.dataset.size).toBe("lg");
    expect(b.dataset.active).toBe("true");
  });
});

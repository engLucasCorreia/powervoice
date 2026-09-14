import { createRawSnippet, flushSync } from "svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import Tooltip from "./Tooltip.svelte";
import { placeTooltip, resetTooltipsForTest } from "./tooltip";
import { byTestId, key, render, type Rendered } from "./testing";

type TriggerProps = { "aria-describedby"?: string };

function trigger(testid: string) {
  return createRawSnippet<[TriggerProps]>((props) => ({
    render: () => {
      const describedBy = props()["aria-describedby"];
      const attr = describedBy ? ` aria-describedby="${describedBy}"` : "";
      return `<button type="button" data-testid="${testid}"${attr}>Play</button>`;
    },
  }));
}

function pointer(el: Element, type: string): void {
  el.dispatchEvent(new MouseEvent(type, { bubbles: type !== "pointerenter" && type !== "pointerleave" }));
  flushSync();
}

let rendered: Rendered[] = [];
beforeEach(() => {
  vi.useFakeTimers();
  resetTooltipsForTest();
});
afterEach(() => {
  rendered.forEach((r) => r.cleanup());
  rendered = [];
  vi.useRealTimers();
});

function mountTip(props: Record<string, unknown>, testid = "t"): Rendered {
  const r = render(Tooltip, { text: "Play", children: trigger(testid), ...props } as never);
  rendered.push(r);
  return r;
}

function tip(r: Rendered): HTMLElement {
  const el = r.target.querySelector<HTMLElement>('[role="tooltip"]');
  if (!el) throw new Error("no tooltip element");
  return el;
}

describe("Tooltip", () => {
  it("describes its trigger and starts hidden", () => {
    const r = mountTip({ text: "Play from start" });
    const button = byTestId(r.target, "t");
    const t = tip(r);
    expect(button.getAttribute("aria-describedby")).toBe(t.id);
    expect(t.textContent?.trim()).toBe("Play from start");
    expect(t.hidden).toBe(true);
  });

  it("opens on hover after the delay and closes on leave", () => {
    const r = mountTip({});
    const anchor = r.target.querySelector(".pv-tooltip-anchor")!;
    pointer(anchor, "pointerenter");
    vi.advanceTimersByTime(499);
    flushSync();
    expect(tip(r).hidden).toBe(true);
    vi.advanceTimersByTime(1);
    flushSync();
    expect(tip(r).hidden).toBe(false);
    pointer(anchor, "pointerleave");
    expect(tip(r).hidden).toBe(true);
  });

  it("opens immediately on keyboard focus and closes with Escape", () => {
    const r = mountTip({});
    const button = byTestId(r.target, "t");
    button.dispatchEvent(new FocusEvent("focusin", { bubbles: true }));
    flushSync();
    expect(tip(r).hidden).toBe(false);
    key(button, "Escape");
    expect(tip(r).hidden).toBe(true);
  });

  it("a click (pointerdown) closes it and the focus that follows doesn't reopen it", () => {
    const r = mountTip({});
    const anchor = r.target.querySelector(".pv-tooltip-anchor")!;
    const button = byTestId(r.target, "t");
    pointer(anchor, "pointerenter");
    vi.advanceTimersByTime(500);
    flushSync();
    pointer(button, "pointerdown");
    button.dispatchEvent(new FocusEvent("focusin", { bubbles: true }));
    flushSync();
    expect(tip(r).hidden).toBe(true);
  });

  it("moving to a neighbour right after one closes opens instantly; only one is open", () => {
    const a = mountTip({ text: "Play" }, "a");
    const b = mountTip({ text: "Stop" }, "b");
    const anchorA = a.target.querySelector(".pv-tooltip-anchor")!;
    const anchorB = b.target.querySelector(".pv-tooltip-anchor")!;
    pointer(anchorA, "pointerenter");
    vi.advanceTimersByTime(500);
    flushSync();
    expect(tip(a).hidden).toBe(false);
    pointer(anchorA, "pointerleave");
    pointer(anchorB, "pointerenter");
    vi.advanceTimersByTime(0);
    flushSync();
    expect(tip(b).hidden).toBe(false);
    expect(tip(a).hidden).toBe(true);
  });

  it("shows the shortcut as a Kbd chip", () => {
    const r = mountTip({ shortcut: "Shift+R" });
    expect(tip(r).querySelector("kbd.pv-kbd")?.textContent).toBe("Shift+R");
  });

  it("describe=false leaves the trigger undescribed (IconButton: the label is already its name)", () => {
    const r = mountTip({ describe: false });
    expect(byTestId(r.target, "t").getAttribute("aria-describedby")).toBeNull();
  });

  it("disabled never opens", () => {
    const r = mountTip({ disabled: true });
    const anchor = r.target.querySelector(".pv-tooltip-anchor")!;
    pointer(anchor, "pointerenter");
    vi.advanceTimersByTime(1000);
    flushSync();
    expect(tip(r).hidden).toBe(true);
  });
});

describe("placeTooltip", () => {
  const viewport = { width: 1280, height: 720 };
  const tipSize = { width: 100, height: 24 };

  it("centres below the anchor", () => {
    const p = placeTooltip({ left: 200, top: 10, width: 28, height: 28 }, tipSize, viewport, "bottom");
    expect(p).toEqual({ left: 164, top: 44, placement: "bottom" });
  });

  it("flips above near the bottom edge and clamps at the right edge", () => {
    const p = placeTooltip({ left: 1270, top: 700, width: 10, height: 10 }, tipSize, viewport, "bottom");
    expect(p.placement).toBe("top");
    expect(p.top).toBe(700 - 6 - 24);
    expect(p.left).toBe(1280 - 8 - 100);
  });

  it("flips below when there is no room above", () => {
    const p = placeTooltip({ left: 0, top: 2, width: 28, height: 28 }, tipSize, viewport, "top");
    expect(p.placement).toBe("bottom");
    expect(p.left).toBe(8);
  });
});

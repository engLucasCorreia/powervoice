import { flushSync, mount, unmount } from "svelte";
import { afterEach, describe, expect, it, vi } from "vitest";
import Splitter from "./Splitter.svelte";

let cleanupTarget: HTMLElement | null = null;
let cleanupApp: ReturnType<typeof mount> | null = null;

afterEach(() => {
  if (cleanupApp) {
    unmount(cleanupApp);
    cleanupApp = null;
  }
  cleanupTarget?.remove();
  cleanupTarget = null;
});

function mountSplitter(props: Record<string, unknown>): HTMLElement {
  const target = document.createElement("div");
  document.body.appendChild(target);
  cleanupTarget = target;
  cleanupApp = mount(Splitter, { target, props: props as never });
  flushSync();
  return target;
}

describe("Splitter (H-24 items 1/2: draggable, keyboard-accessible)", () => {
  it("is a role=separator with the right aria-orientation", () => {
    const target = mountSplitter({
      orientation: "vertical",
      ariaLabel: "Resize Markers",
      testid: "s1",
      onDrag: () => {},
      onReset: () => {},
      onStep: () => {},
    });
    const el = target.querySelector('[data-testid="s1"]')!;
    expect(el.getAttribute("role")).toBe("separator");
    expect(el.getAttribute("aria-orientation")).toBe("vertical");
    expect(el.getAttribute("tabindex")).toBe("0");
  });

  it("reports the pointer delta on drag, resetting the anchor each move (vertical: clientX)", () => {
    const drags: number[] = [];
    let ended = false;
    const target = mountSplitter({
      orientation: "vertical",
      ariaLabel: "Resize",
      testid: "s1",
      onDrag: (d: number) => drags.push(d),
      onDragEnd: () => (ended = true),
      onReset: () => {},
      onStep: () => {},
    });
    const el = target.querySelector('[data-testid="s1"]') as HTMLElement;
    el.setPointerCapture = () => {};
    el.releasePointerCapture = () => {};
    el.dispatchEvent(new PointerEvent("pointerdown", { clientX: 100, bubbles: true }));
    el.dispatchEvent(new PointerEvent("pointermove", { clientX: 130, bubbles: true }));
    el.dispatchEvent(new PointerEvent("pointermove", { clientX: 150, bubbles: true }));
    el.dispatchEvent(new PointerEvent("pointerup", { clientX: 150, bubbles: true }));
    expect(drags).toEqual([30, 20]);
    expect(ended).toBe(true);
  });

  it("uses clientY for a horizontal splitter", () => {
    const drags: number[] = [];
    const target = mountSplitter({
      orientation: "horizontal",
      ariaLabel: "Resize dock",
      testid: "s2",
      onDrag: (d: number) => drags.push(d),
      onReset: () => {},
      onStep: () => {},
    });
    const el = target.querySelector('[data-testid="s2"]') as HTMLElement;
    el.setPointerCapture = () => {};
    el.releasePointerCapture = () => {};
    el.dispatchEvent(new PointerEvent("pointerdown", { clientY: 200, bubbles: true }));
    el.dispatchEvent(new PointerEvent("pointermove", { clientY: 170, bubbles: true }));
    el.dispatchEvent(new PointerEvent("pointerup", { clientY: 170, bubbles: true }));
    expect(drags).toEqual([-30]);
  });

  it("ignores pointermove before a pointerdown", () => {
    const drags: number[] = [];
    const target = mountSplitter({
      orientation: "vertical",
      ariaLabel: "Resize",
      testid: "s1",
      onDrag: (d: number) => drags.push(d),
      onReset: () => {},
      onStep: () => {},
    });
    const el = target.querySelector('[data-testid="s1"]') as HTMLElement;
    el.dispatchEvent(new PointerEvent("pointermove", { clientX: 130, bubbles: true }));
    expect(drags).toEqual([]);
  });

  it("double-click calls onReset", () => {
    const onReset = vi.fn();
    const target = mountSplitter({
      orientation: "vertical",
      ariaLabel: "Resize",
      testid: "s1",
      onDrag: () => {},
      onReset,
      onStep: () => {},
    });
    target
      .querySelector('[data-testid="s1"]')!
      .dispatchEvent(new MouseEvent("dblclick", { bubbles: true }));
    expect(onReset).toHaveBeenCalledOnce();
  });

  it("ArrowRight/ArrowLeft step a vertical splitter; ArrowUp/ArrowDown step a horizontal one", () => {
    const steps: (1 | -1)[] = [];
    const target = mountSplitter({
      orientation: "vertical",
      ariaLabel: "Resize",
      testid: "s1",
      onDrag: () => {},
      onReset: () => {},
      onStep: (d: 1 | -1) => steps.push(d),
    });
    const el = target.querySelector('[data-testid="s1"]') as HTMLElement;
    el.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowRight", bubbles: true }));
    el.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowLeft", bubbles: true }));
    // Vertical splitters ignore vertical arrow keys.
    el.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowUp", bubbles: true }));
    expect(steps).toEqual([1, -1]);
  });

  it("renders a collapse toggle only when collapsible, and it calls onToggleCollapse", () => {
    const onToggleCollapse = vi.fn();
    const target = mountSplitter({
      orientation: "vertical",
      ariaLabel: "Resize",
      testid: "s1",
      collapsible: true,
      onDrag: () => {},
      onReset: () => {},
      onStep: () => {},
      onToggleCollapse,
    });
    const toggle = target.querySelector('[data-testid="s1-collapse"]') as HTMLButtonElement;
    expect(toggle).not.toBeNull();
    toggle.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    expect(onToggleCollapse).toHaveBeenCalledOnce();
  });

  it("renders no collapse toggle when collapsible is false (default)", () => {
    const target = mountSplitter({
      orientation: "vertical",
      ariaLabel: "Resize",
      testid: "s1",
      onDrag: () => {},
      onReset: () => {},
      onStep: () => {},
    });
    expect(target.querySelector('[data-testid="s1-collapse"]')).toBeNull();
  });
});

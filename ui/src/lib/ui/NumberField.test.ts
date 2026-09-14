import { flushSync } from "svelte";
import { afterEach, describe, expect, it, vi } from "vitest";
import NumberField from "./NumberField.svelte";
import { key, render, type Rendered } from "./testing";

let r: Rendered | null = null;
afterEach(() => {
  r?.cleanup();
  r = null;
});

function input(root: HTMLElement): HTMLInputElement {
  const el = root.querySelector<HTMLInputElement>('input[role="spinbutton"]');
  if (!el) throw new Error("no spinbutton");
  return el;
}

function type(el: HTMLInputElement, text: string): void {
  el.value = text;
  el.dispatchEvent(new Event("input", { bubbles: true }));
  flushSync();
}

describe("NumberField", () => {
  it("is a labelled spinbutton showing the value (true minus) with its unit", () => {
    r = render(NumberField, { value: -1, min: -60, max: 0, step: 0.1, unit: "dB", label: "Target" });
    const el = input(r.target);
    expect(el.labels?.[0]?.textContent).toBe("Target");
    expect(el.value).toBe("−1.0");
    expect(el.getAttribute("aria-valuenow")).toBe("-1");
    expect(el.getAttribute("aria-valuemin")).toBe("-60");
    expect(el.getAttribute("aria-valuemax")).toBe("0");
    expect(el.getAttribute("aria-valuetext")).toBe("−1.0 dB");
    expect(r.target.querySelector(".unit")?.textContent).toBe("dB");
  });

  it("Enter commits typed text, accepting − and a trailing unit", () => {
    const onchange = vi.fn();
    r = render(NumberField, { value: -1, min: -60, max: 0, step: 0.1, unit: "dB", label: "Target", onchange });
    const el = input(r.target);
    type(el, "−3.5 dB");
    key(el, "Enter");
    expect(onchange).toHaveBeenCalledWith(-3.5);
    expect(el.value).toBe("−3.5");
  });

  it("out-of-range input clamps on commit", () => {
    const onchange = vi.fn();
    r = render(NumberField, { value: -1, min: -60, max: 0, step: 0.1, label: "Target", onchange });
    const el = input(r.target);
    type(el, "6");
    key(el, "Enter");
    expect(onchange).toHaveBeenCalledWith(0);
  });

  it("garbage is flagged invalid with a message, and doesn't commit", () => {
    const onchange = vi.fn();
    r = render(NumberField, { value: -1, min: -60, max: 0, step: 0.1, unit: "dB", label: "Target", onchange });
    const el = input(r.target);
    type(el, "loud");
    expect(el.getAttribute("aria-invalid")).toBe("true");
    const msgId = el.getAttribute("aria-describedby");
    expect(document.getElementById(msgId!)?.textContent).toBe("Enter a number from −60.0\u00a0dB to 0.0\u00a0dB");
    key(el, "Enter");
    expect(onchange).not.toHaveBeenCalled();
  });

  it("Escape reverts the draft", () => {
    r = render(NumberField, { value: 12, min: 0, max: 100, label: "Crossfade", unit: "ms" });
    const el = input(r.target);
    type(el, "40");
    key(el, "Escape");
    expect(el.value).toBe("12");
    expect(el.getAttribute("aria-invalid")).toBeNull();
  });

  it("ArrowUp/Down step (Shift ×10) and commit immediately", () => {
    const onchange = vi.fn();
    r = render(NumberField, { value: 10, min: 0, max: 100, step: 1, label: "Crossfade", onchange });
    const el = input(r.target);
    key(el, "ArrowUp");
    expect(onchange).toHaveBeenLastCalledWith(11);
    key(el, "ArrowDown", { shiftKey: true });
    expect(onchange).toHaveBeenLastCalledWith(1);
    expect(el.value).toBe("1");
  });

  it("blur commits a valid draft and reverts an invalid one", () => {
    const onchange = vi.fn();
    r = render(NumberField, { value: 1, min: 0, max: 10, step: 0.1, label: "Pre-roll", unit: "s", onchange });
    const el = input(r.target);
    type(el, "2.5");
    el.dispatchEvent(new FocusEvent("blur"));
    el.dispatchEvent(new FocusEvent("focusout", { bubbles: true }));
    flushSync();
    expect(onchange).toHaveBeenCalledWith(2.5);
    type(el, "x");
    el.dispatchEvent(new FocusEvent("blur"));
    el.dispatchEvent(new FocusEvent("focusout", { bubbles: true }));
    flushSync();
    expect(el.value).toBe("2.5");
  });

  it("disabled disables the input", () => {
    r = render(NumberField, { value: 1, label: "Pre-roll", disabled: true });
    expect(input(r.target).disabled).toBe(true);
  });
});

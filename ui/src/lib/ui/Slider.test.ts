import { flushSync } from "svelte";
import { afterEach, describe, expect, it, vi } from "vitest";
import Slider from "./Slider.svelte";
import { key, render, type Rendered } from "./testing";

let r: Rendered | null = null;
afterEach(() => {
  r?.cleanup();
  r = null;
});

function slider(root: HTMLElement): HTMLElement {
  const el = root.querySelector<HTMLElement>('[role="slider"]');
  if (!el) throw new Error("no slider");
  return el;
}

describe("Slider", () => {
  it("exposes the WAI-ARIA slider contract with a unit-bearing value text", () => {
    r = render(Slider, { value: -6, min: -60, max: 12, step: 0.5, unit: "dB", label: "Threshold" });
    const s = slider(r.target);
    expect(s.getAttribute("aria-label")).toBe("Threshold");
    expect(s.getAttribute("aria-valuemin")).toBe("-60");
    expect(s.getAttribute("aria-valuemax")).toBe("12");
    expect(s.getAttribute("aria-valuenow")).toBe("-6");
    expect(s.getAttribute("aria-valuetext")).toBe("−6.0 dB");
    expect(s.tabIndex).toBe(0);
    expect(r.target.querySelector(".value")?.textContent).toBe("−6.0 dB");
  });

  it("arrows step, Shift+arrows and PageUp big-step, Home/End jump — each commits", () => {
    const oninput = vi.fn();
    const onchange = vi.fn();
    r = render(Slider, { value: 0, min: -12, max: 12, step: 0.5, bigStep: 3, unit: "dB", label: "Gain", oninput, onchange });
    const s = slider(r.target);
    key(s, "ArrowRight");
    expect(onchange).toHaveBeenLastCalledWith(0.5);
    key(s, "ArrowUp", { shiftKey: true });
    expect(onchange).toHaveBeenLastCalledWith(3.5);
    key(s, "PageDown");
    expect(onchange).toHaveBeenLastCalledWith(0.5);
    key(s, "End");
    expect(onchange).toHaveBeenLastCalledWith(12);
    expect(s.getAttribute("aria-valuenow")).toBe("12");
    key(s, "Home");
    expect(onchange).toHaveBeenLastCalledWith(-12);
    expect(oninput).toHaveBeenCalledTimes(5);
  });

  it("double-click resets to the default value", () => {
    const onchange = vi.fn();
    r = render(Slider, { value: 5, min: -12, max: 12, step: 0.5, defaultValue: 0, label: "Gain", onchange });
    slider(r.target).dispatchEvent(new MouseEvent("dblclick", { bubbles: true }));
    flushSync();
    expect(onchange).toHaveBeenCalledWith(0);
  });

  it("dragging maps the pointer to a snapped value and commits on release", () => {
    const oninput = vi.fn();
    const onchange = vi.fn();
    r = render(Slider, { value: 0, min: 0, max: 100, step: 1, label: "Mix", oninput, onchange });
    const s = slider(r.target);
    s.getBoundingClientRect = () => ({ left: 10, top: 0, width: 200, height: 16, right: 210, bottom: 16, x: 10, y: 0, toJSON: () => ({}) });
    const Ctor = (window.PointerEvent ?? MouseEvent) as typeof MouseEvent;
    s.dispatchEvent(new Ctor("pointerdown", { clientX: 110, bubbles: true, button: 0 }));
    flushSync();
    expect(oninput).toHaveBeenLastCalledWith(50);
    s.dispatchEvent(new Ctor("pointermove", { clientX: 160, bubbles: true }));
    flushSync();
    expect(oninput).toHaveBeenLastCalledWith(75);
    expect(onchange).not.toHaveBeenCalled();
    s.dispatchEvent(new Ctor("pointerup", { clientX: 160, bubbles: true }));
    flushSync();
    expect(onchange).toHaveBeenCalledWith(75);
  });

  it("disabled: not focusable, aria-disabled, ignores keys", () => {
    const onchange = vi.fn();
    r = render(Slider, { value: 1, min: 0, max: 10, label: "Mix", disabled: true, onchange });
    const s = slider(r.target);
    expect(s.tabIndex).toBe(-1);
    expect(s.getAttribute("aria-disabled")).toBe("true");
    key(s, "ArrowRight");
    expect(onchange).not.toHaveBeenCalled();
  });

  it("marks bipolar ranges (fill grows from zero) and accepts a custom formatter", () => {
    r = render(Slider, { value: 2000, min: 20, max: 20000, label: "Frequency", format: (v: number) => `${v / 1000} kHz` });
    const s = slider(r.target);
    expect(s.getAttribute("aria-valuetext")).toBe("2 kHz");
    expect(s.dataset.bipolar).toBeUndefined();
    r.cleanup();
    r = render(Slider, { value: 0, min: -12, max: 12, label: "Gain" });
    expect(slider(r.target).dataset.bipolar).toBe("true");
  });
});

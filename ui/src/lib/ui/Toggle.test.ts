import { afterEach, describe, expect, it, vi } from "vitest";
import Toggle from "./Toggle.svelte";
import { click, render, type Rendered } from "./testing";

let r: Rendered | null = null;
afterEach(() => {
  r?.cleanup();
  r = null;
});

function sw(root: HTMLElement): HTMLButtonElement {
  const el = root.querySelector<HTMLButtonElement>('[role="switch"]');
  if (!el) throw new Error("no switch");
  return el;
}

describe("Toggle (switch)", () => {
  it("is a switch named by its visible label", () => {
    r = render(Toggle, { label: "Peak hold" });
    const s = sw(r.target);
    const labelId = s.getAttribute("aria-labelledby");
    expect(labelId).toBeTruthy();
    expect(document.getElementById(labelId!)?.textContent).toBe("Peak hold");
    expect(s.getAttribute("aria-checked")).toBe("false");
  });

  it("clicking the switch or its label toggles and reports the value", () => {
    const onchange = vi.fn();
    r = render(Toggle, { label: "Hear original", onchange });
    click(sw(r.target));
    expect(sw(r.target).getAttribute("aria-checked")).toBe("true");
    expect(onchange).toHaveBeenLastCalledWith(true);
    click(r.target.querySelector("label")!);
    expect(sw(r.target).getAttribute("aria-checked")).toBe("false");
    expect(onchange).toHaveBeenLastCalledWith(false);
  });

  it("disabled doesn't toggle", () => {
    const onchange = vi.fn();
    r = render(Toggle, { label: "Peak hold", disabled: true, onchange });
    click(sw(r.target));
    expect(onchange).not.toHaveBeenCalled();
    expect(sw(r.target).disabled).toBe(true);
  });

  it("links an optional description", () => {
    r = render(Toggle, { label: "Pre-roll at cursor", description: "Starts playback before the cursor", checked: true });
    const s = sw(r.target);
    expect(s.getAttribute("aria-checked")).toBe("true");
    const descId = s.getAttribute("aria-describedby");
    expect(document.getElementById(descId!)?.textContent).toBe("Starts playback before the cursor");
  });
});

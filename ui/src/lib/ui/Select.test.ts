import { flushSync } from "svelte";
import { afterEach, describe, expect, it, vi } from "vitest";
import Select from "./Select.svelte";
import { byTestId, render, type Rendered } from "./testing";

const FLOORS = [
  { value: -120, label: "−120 dB" },
  { value: -96, label: "−96 dB" },
  { value: -72, label: "−72 dB", disabled: true },
];

let r: Rendered | null = null;
afterEach(() => {
  r?.cleanup();
  r = null;
});

describe("Select (native, styled)", () => {
  it("is a native select labelled by its visible label, showing the current value", () => {
    r = render(Select, { options: FLOORS, value: -96, label: "Floor", testid: "s" });
    const select = byTestId<HTMLSelectElement>(r.target, "s");
    expect(select.tagName).toBe("SELECT");
    expect(select.labels?.[0]?.textContent).toBe("Floor");
    expect(select.selectedOptions[0]?.textContent).toBe("−96 dB");
  });

  it("change reports the typed option value", () => {
    const onchange = vi.fn();
    r = render(Select, { options: FLOORS, value: -120, label: "Floor", onchange, testid: "s" });
    const select = byTestId<HTMLSelectElement>(r.target, "s");
    select.value = "1";
    select.dispatchEvent(new Event("change", { bubbles: true }));
    flushSync();
    expect(onchange).toHaveBeenCalledWith(-96);
  });

  it("disables the control and individual options", () => {
    r = render(Select, { options: FLOORS, value: -120, label: "Floor", disabled: true, testid: "s" });
    const select = byTestId<HTMLSelectElement>(r.target, "s");
    expect(select.disabled).toBe(true);
    expect(select.options[2]?.disabled).toBe(true);
  });

  it("a hidden label still names the control", () => {
    r = render(Select, { options: FLOORS, value: -120, label: "Floor", hideLabel: true, testid: "s" });
    const label = byTestId<HTMLSelectElement>(r.target, "s").labels?.[0];
    expect(label?.textContent).toBe("Floor");
    expect(label?.classList.contains("visually-hidden")).toBe(true);
  });
});

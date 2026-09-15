import { flushSync, mount, unmount } from "svelte";
import { afterEach, describe, expect, it } from "vitest";
import GainReductionMeter from "./GainReductionMeter.svelte";

/** H-03: the slot-header gain-reduction meter (SPEC-017 §2.3 "Meter", 0 … −24 dB scale). */

let mounted: ReturnType<typeof mount> | null = null;

afterEach(() => {
  if (mounted) {
    unmount(mounted);
    mounted = null;
  }
  document.body.innerHTML = "";
});

function render(value: number | undefined): HTMLElement {
  const target = document.createElement("div");
  document.body.appendChild(target);
  mounted = mount(GainReductionMeter, {
    target,
    props: { value, min: -24, max: 0, name: "Gain reduction" },
  });
  flushSync();
  return target;
}

function fill(root: HTMLElement): string {
  return root.querySelector<HTMLElement>("[data-testid=rack-slot-gr-fill]")?.style.width ?? "";
}

function readout(root: HTMLElement): string {
  return root.querySelector("[data-testid=rack-slot-gr-value]")?.textContent ?? "";
}

describe("GainReductionMeter", () => {
  it("shows the reduction as a bar from 0 dB and as a value", () => {
    const root = render(-6);
    expect(readout(root)).toBe("−6.0");
    expect(fill(root)).toBe("25%");
    const meter = root.querySelector("[role=meter]");
    expect(meter?.getAttribute("aria-valuenow")).toBe("-6");
    expect(meter?.getAttribute("aria-label")).toContain("Gain reduction");
  });

  it("rests at 0 dB without a value and clamps to the scale", () => {
    let root = render(undefined);
    expect(readout(root)).toBe("0.0");
    expect(fill(root)).toBe("0%");
    unmount(mounted!);
    mounted = null;
    root = render(-40);
    expect(readout(root)).toBe("−24.0");
    expect(fill(root)).toBe("100%");
  });
});

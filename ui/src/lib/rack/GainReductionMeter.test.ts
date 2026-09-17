import { flushSync, mount, unmount } from "svelte";
import { afterEach, describe, expect, it } from "vitest";
import GainReductionMeter from "./GainReductionMeter.svelte";

/**
 * H-03/H-77: the gain-reduction meter (SPEC-016 §2.6, SPEC-017 §2.3 "Meter"). The true-peak
 * limiter's channel declares 0 … −24 dB, which is narrower than SPEC-016's 0 … −30 scale, so it
 * keeps its own; past the scale the bar pins and the readout keeps the value, and the channel's
 * own floor reads "≤ −24.0 dB".
 */

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
    expect(readout(root)).toBe("−6.0\u00a0dB");
    expect(fill(root)).toBe("25%");
    const meter = root.querySelector("[role=meter]");
    expect(meter?.getAttribute("aria-valuenow")).toBe("-6");
    expect(meter?.getAttribute("aria-label")).toContain("Gain reduction");
  });

  it("rests at 0 dB without a value and clamps to the scale", () => {
    let root = render(undefined);
    expect(readout(root)).toBe("0.0\u00a0dB");
    expect(fill(root)).toBe("0%");
    unmount(mounted!);
    mounted = null;
    root = render(-40);
    // Past the channel's floor: the bar pins and the readout says so (SPEC-016 §2.6).
    expect(readout(root)).toBe("≤ −24.0\u00a0dB");
    expect(fill(root)).toBe("100%");
  });
});

import { afterEach, describe, expect, it } from "vitest";
import Readout from "./Readout.svelte";
import { render, type Rendered } from "./testing";

let r: Rendered | null = null;
afterEach(() => {
  r?.cleanup();
  r = null;
});

describe("Readout (value + unit in tabular figures)", () => {
  it("shows label, a minus-signed value and its unit", () => {
    r = render(Readout, { label: "Integrated", value: -23, unit: "LUFS", decimals: 1, testid: "ro" });
    const el = r.target.querySelector('[data-testid="ro"]') as HTMLElement;
    expect(el.querySelector(".label")?.textContent).toBe("Integrated");
    expect(el.querySelector(".value")?.textContent).toBe("−23.0");
    expect(el.querySelector(".unit")?.textContent).toBe("LUFS");
  });

  it("renders silence as −∞ and exposes the tone", () => {
    r = render(Readout, { value: Number.NEGATIVE_INFINITY, unit: "dBFS", decimals: 1, tone: "danger" });
    const el = r.target.querySelector(".pv-readout") as HTMLElement;
    expect(el.querySelector(".value")?.textContent).toBe("−∞");
    expect(el.dataset.tone).toBe("danger");
  });

  it("accepts preformatted text (timecode) and signed gains", () => {
    r = render(Readout, { text: "0:05.250", size: "xl" });
    expect(r.target.querySelector(".value")?.textContent).toBe("0:05.250");
    expect((r.target.querySelector(".pv-readout") as HTMLElement).dataset.size).toBe("xl");
    r.cleanup();
    r = render(Readout, { value: 3, unit: "dB", decimals: 1, signed: true });
    expect(r.target.querySelector(".value")?.textContent).toBe("+3.0");
  });
});

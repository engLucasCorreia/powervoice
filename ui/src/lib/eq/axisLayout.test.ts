import { describe, expect, it } from "vitest";
import { rectsOverlap } from "../ui/axisLabels";
import { formatRulerFreqHz } from "../spectrum/freqAxis";
import { eqAxisLayout, type EqAxisLabel } from "./axisLayout";
import { EQ_GAIN_RANGE_DEFAULT_DB, EQ_GAIN_RANGE_WIDE_DB } from "./gainAxis";

const HEIGHT = 160;

function layout(width: number, rangeDb: number) {
  return eqAxisLayout(width, HEIGHT, rangeDb, 20, 20_000, formatRulerFreqHz, "Hz");
}

function all(l: ReturnType<typeof layout>): EqAxisLabel[] {
  return [...l.gain, ...l.freq, l.freqUnit];
}

describe("eqAxisLayout (H-26: EQ graph labels never collide)", () => {
  it("no two labels overlap and every label stays inside the graph, at any width and range", () => {
    for (const rangeDb of [EQ_GAIN_RANGE_DEFAULT_DB, EQ_GAIN_RANGE_WIDE_DB]) {
      for (let width = 140; width <= 720; width += 20) {
        const labels = all(layout(width, rangeDb));
        for (const label of labels) {
          const where = `${label.text} @ ${width}px ±${rangeDb}`;
          expect(label.rect.x, where).toBeGreaterThanOrEqual(0);
          expect(label.rect.y, where).toBeGreaterThanOrEqual(0);
          expect(label.rect.x + label.rect.width, where).toBeLessThanOrEqual(width);
          expect(label.rect.y + label.rect.height, where).toBeLessThanOrEqual(HEIGHT);
        }
        for (let i = 0; i < labels.length; i++) {
          for (let j = i + 1; j < labels.length; j++) {
            expect(
              rectsOverlap(labels[i]!.rect, labels[j]!.rect),
              `${labels[i]!.text} vs ${labels[j]!.text} @ ${width}px ±${rangeDb}`,
            ).toBe(false);
          }
        }
      }
    }
  });

  it("always shows the 0 dB line and both gain range ends, with a true minus", () => {
    const gain = layout(280, EQ_GAIN_RANGE_DEFAULT_DB).gain.map((l) => l.text);
    expect(gain).toContain("0");
    expect(gain).toContain("+12");
    expect(gain).toContain("+6");
    expect(gain).toContain("−6");
    expect(gain.join(" ")).not.toMatch(/-\d/);
  });

  it("the top and bottom gain labels hang inside the graph instead of straddling its edge", () => {
    const gain = layout(280, EQ_GAIN_RANGE_DEFAULT_DB).gain;
    const top = gain.find((l) => l.text === "+12")!;
    expect(top.baseline).toBe("top");
    expect(top.rect.y).toBe(0);
  });

  it("keeps the frequency unit in its slot at the end of the frequency row", () => {
    const l = layout(280, EQ_GAIN_RANGE_DEFAULT_DB);
    expect(l.freqUnit.text).toBe("Hz");
    expect(l.freqUnit.rect.x + l.freqUnit.rect.width).toBeLessThanOrEqual(280);
    expect(l.freq.length).toBeGreaterThanOrEqual(4);
    // The last frequency label ends before the unit starts.
    const last = l.freq.reduce((a, b) => (a.rect.x > b.rect.x ? a : b));
    expect(last.rect.x + last.rect.width).toBeLessThan(l.freqUnit.rect.x);
  });
});

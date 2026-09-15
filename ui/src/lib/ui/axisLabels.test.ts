import { describe, expect, it } from "vitest";
import {
  edgeAlign,
  estimateLabelWidthPx,
  fitAxisLabels,
  fitGutterLabels,
  labelSpan,
  rectsOverlap,
  spansOverlap,
} from "./axisLabels";

describe("axis-label collision maths", () => {
  it("computes a label's span from its anchor and alignment", () => {
    expect(labelSpan(50, 20, "center")).toEqual({ start: 40, end: 60 });
    expect(labelSpan(50, 20, "start")).toEqual({ start: 50, end: 70 });
    expect(labelSpan(50, 20, "end")).toEqual({ start: 30, end: 50 });
  });

  it("detects overlaps, honouring a minimum gap", () => {
    expect(spansOverlap({ start: 0, end: 10 }, { start: 10, end: 20 })).toBe(false);
    expect(spansOverlap({ start: 0, end: 10 }, { start: 9, end: 20 })).toBe(true);
    expect(spansOverlap({ start: 0, end: 10 }, { start: 11, end: 20 }, 2)).toBe(true);
    expect(rectsOverlap({ x: 0, y: 0, width: 10, height: 10 }, { x: 5, y: 5, width: 10, height: 10 })).toBe(true);
    expect(rectsOverlap({ x: 0, y: 0, width: 10, height: 10 }, { x: 5, y: 12, width: 10, height: 10 })).toBe(false);
  });

  it("edge-aligns labels near the ends of the axis so they aren't cut off", () => {
    expect(edgeAlign(0, 200, 12)).toBe("start");
    expect(edgeAlign(100, 200, 12)).toBe("center");
    expect(edgeAlign(200, 200, 12)).toBe("end");
  });

  it("drops labels that would sit on a reserved unit slot", () => {
    const labels = [0, 20, 40, 60].map((pos) => ({ pos, size: 12 }));
    const fitted = fitAxisLabels(labels, { length: 60, reserved: [{ start: 54, end: 60 }] });
    // 60 is end-aligned to [48, 60] and collides with the slot; the rest stay.
    expect(fitted.map((l) => l.pos)).toEqual([0, 20, 40]);
  });

  it("drops a label that would overlap one kept before it", () => {
    const labels = [
      { pos: 0, size: 20 },
      { pos: 15, size: 20 },
      { pos: 40, size: 20 },
    ];
    expect(fitAxisLabels(labels, { length: 100 }).map((l) => l.pos)).toEqual([0, 40]);
  });

  it("lets the caller's order decide which label wins", () => {
    const labels = [
      { pos: 15, size: 20, id: "zero-line" },
      { pos: 0, size: 20, id: "edge" },
    ];
    expect(fitAxisLabels(labels, { length: 100 }).map((l) => l.id)).toEqual(["zero-line"]);
  });

  it("never returns a label outside the axis", () => {
    const fitted = fitAxisLabels([{ pos: 5, size: 20, align: "end" as const }], { length: 100 });
    expect(fitted).toEqual([]);
  });

  it("estimates short label widths generously", () => {
    expect(estimateLabelWidthPx("−120", 10)).toBeGreaterThanOrEqual(24);
    expect(estimateLabelWidthPx("20k", 10)).toBeLessThan(estimateLabelWidthPx("1:02:03", 10));
  });

  describe("fitGutterLabels (vertical rulers with a unit in the corner)", () => {
    const options = { length: 200, width: 48, fontPx: 10, lineHeightPx: 12, unit: { text: "Hz", fontPx: 10 } };

    it("keeps a narrow top label beside the unit and edge-aligns it so it isn't cut off", () => {
      const fitted = fitGutterLabels([{ pos: 0, text: "8k" }], options);
      expect(fitted).toHaveLength(1);
      expect(fitted[0]!.align).toBe("start");
      expect(fitted[0]!.span.start).toBeGreaterThanOrEqual(0);
    });

    it("drops a top label wide enough to touch the unit", () => {
      expect(fitGutterLabels([{ pos: 0, text: "12.5k" }], { ...options, width: 36 })).toEqual([]);
    });

    it("drops labels that would overlap each other and keeps the bottom one inside", () => {
      const fitted = fitGutterLabels(
        [
          { pos: 100, text: "1k" },
          { pos: 106, text: "2k" },
          { pos: 200, text: "20" },
        ],
        options,
      );
      expect(fitted.map((l) => l.text)).toEqual(["1k", "20"]);
      expect(fitted[1]!.align).toBe("end");
      expect(fitted[1]!.span.end).toBeLessThanOrEqual(200);
    });
  });
});

import { describe, expect, it } from "vitest";
import { rectsOverlap } from "../ui/axisLabels";
import { placePeakLabels } from "./peakLabels";

const box = (x: number, y: number, width = 80, height = 26) => ({ x, y, width, height });

describe("peak label placement (H-42)", () => {
  it("puts a lone label above its marker", () => {
    const [p] = placePeakLabels([box(200, 100)], { width: 600, height: 200 });
    expect(p?.spot).toBe("above");
    expect(p?.rect).toEqual({ x: 160, y: 68, width: 80, height: 26 });
  });

  it("never lets two labels collide, and keeps them inside the plot", () => {
    // Five peaks crowded into 150 px, some near the edges.
    const items = [box(10, 40), box(60, 50), box(90, 45), box(120, 60), box(590, 20)];
    const placed = placePeakLabels(items, { width: 600, height: 160 });
    expect(placed.length).toBeGreaterThanOrEqual(3);
    for (const p of placed) {
      expect(p.rect.x).toBeGreaterThanOrEqual(0);
      expect(p.rect.y).toBeGreaterThanOrEqual(0);
      expect(p.rect.x + p.rect.width).toBeLessThanOrEqual(600);
      expect(p.rect.y + p.rect.height).toBeLessThanOrEqual(160);
    }
    for (let i = 0; i < placed.length; i++) {
      for (let j = i + 1; j < placed.length; j++) {
        expect(rectsOverlap(placed[i]!.rect, placed[j]!.rect, 3)).toBe(false);
      }
    }
  });

  it("honours priority: the first label always gets its best spot", () => {
    const items = [box(100, 80), box(110, 80)];
    const placed = placePeakLabels(items, { width: 400, height: 200 });
    expect(placed[0]?.item).toBe(items[0]);
    expect(placed[0]?.spot).toBe("above");
    expect(placed[1]?.spot).not.toBe("above");
  });

  it("avoids reserved rectangles and drops what can't fit", () => {
    const reserved = [{ x: 0, y: 0, width: 400, height: 60 }];
    const [p] = placePeakLabels([box(200, 70)], { width: 400, height: 200, reserved });
    expect(p?.spot).toBe("below");
    expect(placePeakLabels([box(20, 10, 300, 190)], { width: 100, height: 100 })).toEqual([]);
  });
});

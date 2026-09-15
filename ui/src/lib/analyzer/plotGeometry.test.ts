import { describe, expect, it } from "vitest";
import { allBelow, columnize, emaAlpha, levelAt, nearestIndex, smoothLevels } from "./plotGeometry";

describe("spectrum plot geometry (H-42)", () => {
  it("keeps the loudest point of each pixel column", () => {
    const freqs = Float64Array.from({ length: 1001 }, (_, k) => k * 10); // 0 … 10 kHz
    const levels = Float32Array.from({ length: 1001 }, (_, k) => (k === 503 ? 0 : -60));
    const x = (f: number) => (f / 10_000) * 100; // 100 px: 10 bins per column
    const pts = columnize(freqs, levels, x, 0, 10_000);
    expect(pts.length).toBeLessThanOrEqual(101);
    expect(pts.length).toBeGreaterThanOrEqual(100);
    const peak = pts.find((p) => p.db === 0)!;
    expect(peak.x).toBeCloseTo(50.3, 5);
    for (let i = 1; i < pts.length; i++) {
      expect(pts[i]!.x).toBeGreaterThan(pts[i - 1]!.x);
    }
  });

  it("passes a sparse curve through point for point and clips to the range (one point of margin)", () => {
    const freqs = [20, 40, 80, 160, 320, 640];
    const levels = [-10, -20, -30, -40, -50, -60];
    const x = (f: number) => Math.log2(f / 20) * 100;
    expect(columnize(freqs, levels, x, 20, 640).length).toBe(6);
    const clipped = columnize(freqs, levels, x, 100, 200);
    expect(clipped.map((p) => p.db)).toEqual([-30, -40, -50]);
    // A log axis maps 0 Hz to −∞: skipped.
    expect(columnize([0, 100], [0, -1], (f) => Math.log(f), 0, 100).length).toBe(1);
  });

  it("smooths in the power domain and restarts on a size change", () => {
    const a = smoothLevels(null, [0, -Infinity], 0.5);
    expect(Array.from(a)).toEqual([0, -Infinity]);
    const b = smoothLevels(a, [-Infinity, 0], 0.5);
    expect(b[0]).toBeCloseTo(10 * Math.log10(0.5), 5);
    expect(b[1]).toBeCloseTo(10 * Math.log10(0.5), 5);
    expect(smoothLevels(b, [1, 2, 3], 0.1).length).toBe(3);
    expect(emaAlpha(0.1, 0.4)).toBeCloseTo(1 - Math.exp(-0.25), 9);
    expect(emaAlpha(0.1, 0)).toBe(1);
  });

  it("looks levels up by nearest frequency", () => {
    const curve = { freqsHz: [100, 200, 400], levelsDb: [-1, -2, -3] };
    expect(nearestIndex(curve.freqsHz, 140)).toBe(0);
    expect(nearestIndex(curve.freqsHz, 160)).toBe(1);
    expect(nearestIndex(curve.freqsHz, 9000)).toBe(2);
    expect(levelAt(curve, 390)).toBe(-3);
    expect(levelAt({ freqsHz: [], levelsDb: [] }, 1)).toBe(-Infinity);
  });

  it("knows when nothing is visible", () => {
    expect(allBelow([-Infinity, -130], -120)).toBe(true);
    expect(allBelow([-Infinity, -110], -120)).toBe(false);
  });
});

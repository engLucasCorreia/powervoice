import { describe, expect, it } from "vitest";
import { computeDbRange } from "./explainGraphMath";

describe("computeDbRange (H-92: never truncate a real peak)", () => {
  it("gives every finite point headroom above and below, on a clean 10 dB grid", () => {
    const raw = Float32Array.from([-90, -40, -95, -Infinity]);
    const smoothed = Float32Array.from([-88, -45, -90]);
    const [floor, ceil] = computeDbRange(raw, smoothed);
    expect(ceil).toBeGreaterThanOrEqual(-40 + 6);
    expect(floor).toBeLessThanOrEqual(-95 - 6);
    expect(Math.abs(ceil % 10)).toBe(0);
    expect(Math.abs(floor % 10)).toBe(0);
  });

  it("never truncates the loudest real point of either curve", () => {
    const raw = Float32Array.from([-12, -60, -80]);
    const smoothed = Float32Array.from([-70, -75]);
    const [, ceil] = computeDbRange(raw, smoothed);
    expect(ceil).toBeGreaterThan(-12);
  });

  it("keeps a minimum span even for a nearly flat curve", () => {
    const raw = Float32Array.from([-40, -40.2, -39.9]);
    const [floor, ceil] = computeDbRange(raw, raw);
    expect(ceil - floor).toBeGreaterThanOrEqual(30);
  });

  it("falls back to the analyzer's usual range when nothing is finite", () => {
    const silence = Float32Array.from([-Infinity, -Infinity]);
    expect(computeDbRange(silence, silence)).toEqual([-120, 0]);
  });
});

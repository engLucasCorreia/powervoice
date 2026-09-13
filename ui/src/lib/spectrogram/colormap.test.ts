import { describe, expect, it } from "vitest";
import { colorForT, colormapLut, COLORMAP_NAMES, normalizeDb } from "./colormap";

describe("colormap (SPEC-007 §2.5)", () => {
  it("every colormap is a 256-entry RGB LUT", () => {
    for (const name of COLORMAP_NAMES) {
      expect(colormapLut(name).length).toBe(256 * 3);
    }
  });

  it("inferno runs black (t=0) to pale yellow (t=1)", () => {
    const [r0, g0, b0] = colorForT("inferno", 0);
    expect([r0, g0, b0]).toEqual([0, 0, 4]);
    const [r1, g1, b1] = colorForT("inferno", 1);
    expect(r1).toBeGreaterThan(240);
    expect(g1).toBeGreaterThan(240);
    expect(b1).toBeGreaterThan(150);
  });

  it("viridis runs dark purple (t=0) to yellow-green (t=1)", () => {
    const [r0, g0, b0] = colorForT("viridis", 0);
    expect([r0, g0, b0]).toEqual([68, 1, 84]);
    const [r1, g1] = colorForT("viridis", 1);
    expect(r1).toBeGreaterThan(240);
    expect(g1).toBeGreaterThan(220);
  });

  it("grayscale runs black to white with r=g=b throughout", () => {
    for (const t of [0, 0.25, 0.5, 0.75, 1]) {
      const [r, g, b] = colorForT("gray", t);
      expect(r).toBe(g);
      expect(g).toBe(b);
    }
    expect(colorForT("gray", 0)).toEqual([0, 0, 0]);
    expect(colorForT("gray", 1)).toEqual([255, 255, 255]);
  });

  it("clamps t outside [0, 1] to the colormap's end colors", () => {
    expect(colorForT("gray", -5)).toEqual([0, 0, 0]);
    expect(colorForT("gray", 5)).toEqual([255, 255, 255]);
    expect(colorForT("gray", Number.NaN)).toEqual([0, 0, 0]);
  });

  it("normalizes dB against the display floor/ceiling, clamped to [0, 1]", () => {
    expect(normalizeDb(-120, -120, 0)).toBe(0);
    expect(normalizeDb(0, -120, 0)).toBe(1);
    expect(normalizeDb(-60, -120, 0)).toBeCloseTo(0.5, 6);
    expect(normalizeDb(-200, -120, 0)).toBe(0); // below floor clamps
    expect(normalizeDb(20, -120, 0)).toBe(1); // above ceiling clamps
    expect(normalizeDb(-60, 0, -120)).toBe(0); // degenerate range (ceil <= floor)
  });

  it("quantizes to 256 distinct steps (SPEC-007 §2.5)", () => {
    const seen = new Set<string>();
    for (let i = 0; i <= 255; i++) {
      seen.add(colorForT("inferno", i / 255).join(","));
    }
    expect(seen.size).toBeGreaterThan(200); // most steps distinct; some may tie in flat segments
  });
});

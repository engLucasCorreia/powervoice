import { describe, expect, it } from "vitest";
import { spansOverlap } from "../ui/axisLabels";
import {
  HOT_ZONE_DB,
  LOUD_ZONE_DB,
  METER_FLOOR_DB,
  METER_LABEL_GAP_PX,
  meterFraction,
  meterScaleTicks,
} from "./meterScale";

describe("meterFraction", () => {
  it("maps 0 dBFS to 1 and the floor to 0", () => {
    expect(meterFraction(0)).toBe(1);
    expect(meterFraction(METER_FLOOR_DB)).toBe(0);
  });

  it("is linear in dB between the floor and 0", () => {
    expect(meterFraction(METER_FLOOR_DB / 2)).toBeCloseTo(0.5, 6);
  });

  it("clamps beyond either end and treats -Infinity as the floor", () => {
    expect(meterFraction(5)).toBe(1);
    expect(meterFraction(METER_FLOOR_DB - 20)).toBe(0);
    expect(meterFraction(Number.NEGATIVE_INFINITY)).toBe(0);
    expect(meterFraction(Number.NaN)).toBe(0);
  });

  it("places the ticket's colour-zone boundaries at tidy fractions of a -60 dBFS floor", () => {
    // These are exact given METER_FLOOR_DB === -60; if the floor ever changes, this test should
    // fail loudly rather than the zones silently drifting.
    expect(meterFraction(LOUD_ZONE_DB)).toBeCloseTo(0.7, 6);
    expect(meterFraction(HOT_ZONE_DB)).toBeCloseTo(0.95, 6);
  });
});

describe("meterScaleTicks", () => {
  const LINE_H = 12;

  it("returns nothing for a non-positive height", () => {
    expect(meterScaleTicks(0, LINE_H)).toEqual([]);
    expect(meterScaleTicks(-10, LINE_H)).toEqual([]);
  });

  it("always includes 0 dBFS at the top and -∞ pinned to the absolute bottom", () => {
    const ticks = meterScaleTicks(400, LINE_H);
    const zero = ticks.find((t) => t.db === 0);
    const inf = ticks.find((t) => t.db === Number.NEGATIVE_INFINITY);
    expect(zero?.y).toBe(0);
    expect(zero?.label).toBe("0");
    expect(inf?.y).toBe(400);
    expect(inf?.label).toBe("−∞"); // true minus (U+2212), not an ASCII hyphen
  });

  it("never lets any two labels collide, at a generous height", () => {
    const ticks = meterScaleTicks(500, LINE_H);
    for (let i = 0; i < ticks.length; i += 1) {
      for (let j = i + 1; j < ticks.length; j += 1) {
        const a = { start: ticks[i]!.y - LINE_H / 2, end: ticks[i]!.y + LINE_H / 2 };
        const b = { start: ticks[j]!.y - LINE_H / 2, end: ticks[j]!.y + LINE_H / 2 };
        expect(spansOverlap(a, b)).toBe(false);
      }
    }
    // At a generous height the full owner's ladder (plus -∞) should all survive thinning.
    expect(ticks.length).toBe(10);
  });

  it("thins to the highest-priority ticks (loudest first) as the meter gets short, but keeps -∞", () => {
    const ticks = meterScaleTicks(40, LINE_H);
    expect(ticks.some((t) => t.db === 0)).toBe(true);
    expect(ticks.some((t) => t.db === Number.NEGATIVE_INFINITY)).toBe(true);
    expect(ticks.length).toBeLessThan(10);
    for (let i = 0; i < ticks.length; i += 1) {
      for (let j = i + 1; j < ticks.length; j += 1) {
        const a = { start: ticks[i]!.y - LINE_H / 2, end: ticks[i]!.y + LINE_H / 2 };
        const b = { start: ticks[j]!.y - LINE_H / 2, end: ticks[j]!.y + LINE_H / 2 };
        expect(spansOverlap(a, b)).toBe(false);
      }
    }
  });

  it("degenerates gracefully at a tiny height without throwing or colliding", () => {
    expect(() => meterScaleTicks(5, LINE_H)).not.toThrow();
    const ticks = meterScaleTicks(5, LINE_H);
    expect(ticks.some((t) => t.db === Number.NEGATIVE_INFINITY)).toBe(true);
  });

  it("never computes a position from a live meter value — only from height/font metrics", () => {
    // Type-level guarantee, exercised here: calling it twice with the same height always gives
    // the same ticks, because there is no level parameter to vary.
    expect(meterScaleTicks(300, LINE_H)).toEqual(meterScaleTicks(300, LINE_H));
  });

  // H-48 item 2 (owner report: "-12/-18/-24 run together" at short dock heights). At these exact
  // heights the old 2 px default let all three survive the fit only 2 px apart — reproduced by
  // asking for that tighter gap explicitly and confirming it used to keep all three, then
  // asserting today's default (a roomier `METER_LABEL_GAP_PX`) either drops one of them or keeps
  // a real gap between every pair, at every one of these dock heights.
  describe("readable spacing at short dock heights (H-48 item 2)", () => {
    // Heights where a bare 2 px gap used to let -12/-18/-24 all survive the fit, only 2 px apart.
    const CROWDED_HEIGHTS = [160, 170];
    // A wider spread of short-to-generous heights, for the invariants that must hold everywhere.
    const ALL_HEIGHTS = [60, 90, 120, 150, 160, 170, 180, 200, 220];

    it("a bare 2 px gap used to let -12/-18/-24 all survive, touching, at these heights", () => {
      for (const heightPx of CROWDED_HEIGHTS) {
        const dbs = meterScaleTicks(heightPx, LINE_H, 2).map((t) => t.db);
        expect(dbs).toEqual(expect.arrayContaining([-12, -18, -24]));
      }
    });

    it("today's default no longer keeps all three that tight — one is dropped for breathing room", () => {
      for (const heightPx of CROWDED_HEIGHTS) {
        const dbs = meterScaleTicks(heightPx, LINE_H).map((t) => t.db);
        expect(dbs).not.toEqual(expect.arrayContaining([-12, -18, -24]));
      }
    });

    it("keeps at least METER_LABEL_GAP_PX of clear space between every pair of kept labels by default", () => {
      for (const heightPx of ALL_HEIGHTS) {
        const ticks = [...meterScaleTicks(heightPx, LINE_H)].sort((a, b) => a.y - b.y);
        for (let i = 0; i < ticks.length; i += 1) {
          for (let j = i + 1; j < ticks.length; j += 1) {
            const a = { start: ticks[i]!.y - LINE_H / 2, end: ticks[i]!.y + LINE_H / 2 };
            const b = { start: ticks[j]!.y - LINE_H / 2, end: ticks[j]!.y + LINE_H / 2 };
            expect(spansOverlap(a, b, METER_LABEL_GAP_PX - 0.5)).toBe(false);
          }
        }
      }
    });

    it("still always keeps 0 dBFS and -∞, even once other ticks are thinned for readability", () => {
      for (const heightPx of ALL_HEIGHTS) {
        const ticks = meterScaleTicks(heightPx, LINE_H);
        expect(ticks.some((t) => t.db === 0)).toBe(true);
        expect(ticks.some((t) => t.db === Number.NEGATIVE_INFINITY)).toBe(true);
      }
    });
  });
});

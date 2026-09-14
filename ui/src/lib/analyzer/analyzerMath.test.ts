import { describe, expect, it } from "vitest";
import {
  ANALYZER_CEIL_OPTIONS_DB,
  ANALYZER_FLOOR_OPTIONS_DB,
  dbAxisTicks,
  dbForAnalyzerY,
  nearestAnalyzerBand,
  yForAnalyzerDb,
} from "./analyzerMath";

describe("yForAnalyzerDb / dbForAnalyzerY (H-16, SPEC-007 §2.9)", () => {
  it("maps the ceiling to the top and the floor to the bottom", () => {
    expect(yForAnalyzerDb(0, -120, 0, 200)).toBe(0);
    expect(yForAnalyzerDb(-120, -120, 0, 200)).toBe(200);
    expect(yForAnalyzerDb(-60, -120, 0, 200)).toBe(100);
  });

  it("clamps out-of-range dB to the axis edges", () => {
    expect(yForAnalyzerDb(10, -120, 0, 200)).toBe(0);
    expect(yForAnalyzerDb(-200, -120, 0, 200)).toBe(200);
  });

  it("is the exact inverse of dbForAnalyzerY over the whole non-clamped range", () => {
    for (const floorDb of ANALYZER_FLOOR_OPTIONS_DB) {
      for (const ceilDb of ANALYZER_CEIL_OPTIONS_DB) {
        for (const db of [floorDb, floorDb / 2, ceilDb]) {
          const y = yForAnalyzerDb(db, floorDb, ceilDb, 300);
          expect(dbForAnalyzerY(y, floorDb, ceilDb, 300)).toBeCloseTo(db, 6);
        }
      }
    }
  });

  it("degenerate range (ceil <= floor) doesn't divide by zero", () => {
    expect(yForAnalyzerDb(-10, 0, 0, 200)).toBe(200);
    expect(dbForAnalyzerY(50, 0, 0, 200)).toBe(0);
  });
});

describe("nearestAnalyzerBand (H-16, SPEC-007 §4.8 f_k = 20·2^(k/24))", () => {
  const F0 = 20;
  const BANDS_PER_OCTAVE = 24;
  const BAND_COUNT = 246;

  it("finds band 0 at f0", () => {
    expect(nearestAnalyzerBand(F0, F0, BANDS_PER_OCTAVE, BAND_COUNT)).toBe(0);
  });

  it("finds the band nearest a round frequency", () => {
    // 1 kHz is band round(24 * log2(1000/20)) = round(24 * 5.6439) = 135.
    const k = nearestAnalyzerBand(1000, F0, BANDS_PER_OCTAVE, BAND_COUNT);
    expect(k).toBe(135);
    // Its centre should indeed be the closest of its neighbours to 1000 Hz.
    const centre = (band: number) => F0 * 2 ** (band / BANDS_PER_OCTAVE);
    const dist = (band: number) => Math.abs(centre(band) - 1000);
    expect(dist(k)).toBeLessThanOrEqual(dist(k - 1));
    expect(dist(k)).toBeLessThanOrEqual(dist(k + 1));
  });

  it("clamps to [0, bandCount - 1] for out-of-range frequencies", () => {
    expect(nearestAnalyzerBand(1, F0, BANDS_PER_OCTAVE, BAND_COUNT)).toBe(0);
    expect(nearestAnalyzerBand(1_000_000, F0, BANDS_PER_OCTAVE, BAND_COUNT)).toBe(BAND_COUNT - 1);
  });

  it("never throws on degenerate input", () => {
    expect(nearestAnalyzerBand(0, F0, BANDS_PER_OCTAVE, BAND_COUNT)).toBe(0);
    expect(nearestAnalyzerBand(-5, F0, BANDS_PER_OCTAVE, BAND_COUNT)).toBe(0);
    expect(nearestAnalyzerBand(1000, F0, BANDS_PER_OCTAVE, 0)).toBe(0);
  });
});

describe("dbAxisTicks (H-24 item 5: 10 dB spacing, or 20 dB when the pane is too short)", () => {
  it("uses 10 dB steps on a tall pane", () => {
    const ticks = dbAxisTicks(-120, 0, 400, 20);
    expect(ticks.map((t) => t.db)).toEqual([-120, -110, -100, -90, -80, -70, -60, -50, -40, -30, -20, -10, 0]);
    expect(ticks[0]!.y).toBe(400); // floor at the bottom
    expect(ticks[ticks.length - 1]!.y).toBe(0); // ceiling at the top
  });

  it("falls back to 20 dB steps when 10 dB spacing would be tighter than the minimum gap", () => {
    const ticks = dbAxisTicks(-120, 0, 60, 20); // 60px / 12 steps = 5px per 10dB step, too tight
    expect(ticks.map((t) => t.db)).toEqual([-120, -100, -80, -60, -40, -20, 0]);
  });

  it("labels are whole-number dB strings", () => {
    const ticks = dbAxisTicks(-120, 0, 400, 20);
    expect(ticks.every((t) => /^-?\d+$/.test(t.label))).toBe(true);
  });

  it("returns an empty list for a degenerate range or non-positive height", () => {
    expect(dbAxisTicks(0, 0, 400, 20)).toEqual([]);
    expect(dbAxisTicks(-120, 0, 0, 20)).toEqual([]);
  });
});

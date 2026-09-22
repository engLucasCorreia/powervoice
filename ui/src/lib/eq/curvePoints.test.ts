import { describe, expect, it } from "vitest";
import type { ResponseCurveDto } from "../ipc/bindings";
import { xForFreq } from "./freqAxis";
import { yForDb } from "./gainAxis";
import { totalCurveToScreen, totalDbAtFreq } from "./curvePoints";

/**
 * AC-17 (graph draws Rust's curve only): given an arbitrary synthetic shape no filter produces
 * (a sawtooth in dB), the mapped polyline passes through `(freqAxis.x(f_i), gainAxis.y(dB_i))`
 * for every point, within 0.5 device px. The UI never evaluates a filter — this test constructs
 * the "response" out of thin air and checks only the pixel mapping.
 */
describe("totalCurveToScreen (AC-17)", () => {
  it("maps every point of a synthetic sawtooth curve through the axis functions exactly", () => {
    const freqs = [20, 50, 200, 1_000, 5_000, 20_000];
    const sawtooth = freqs.map((_, i) => (i % 2 === 0 ? 10 : -10)); // no real filter does this
    const curve: ResponseCurveDto = {
      freqs_hz: freqs,
      sample_rate_hz: 48_000,
      total_db: sawtooth,
      components_db: [],
    };
    const width = 500;
    const height = 160;
    const fLo = 20;
    const fHi = 20_000;
    const rangeDb = 12;

    const points = totalCurveToScreen(curve, width, height, fLo, fHi, rangeDb);
    expect(points).toHaveLength(freqs.length);
    freqs.forEach((f, i) => {
      const expectedX = xForFreq(f, width, fLo, fHi);
      const expectedY = yForDb(sawtooth[i] ?? 0, height, rangeDb);
      const point = points[i]!;
      expect(Math.abs(point.x - expectedX)).toBeLessThanOrEqual(0.5);
      expect(Math.abs(point.y - expectedY)).toBeLessThanOrEqual(0.5);
    });
    // The sawtooth really is drawn as a sawtooth (alternating high/low), not smoothed away —
    // proof the mapping doesn't run any filter math of its own.
    expect(points[0]!.y).toBeLessThan(points[1]!.y); // +10 dB draws higher (smaller y) than -10 dB
  });

  it("is empty for an empty curve", () => {
    const curve: ResponseCurveDto = {
      freqs_hz: [],
      sample_rate_hz: 48_000,
      total_db: [],
      components_db: [],
    };
    expect(totalCurveToScreen(curve, 500, 160, 20, 20_000, 12)).toEqual([]);
  });
});

/**
 * H-111 (SPEC-015 §2.6.4 "the total response there (linearly interpolated between the returned
 * curve points)"): the cursor readout's own math, tested independently of any canvas.
 */
describe("totalDbAtFreq (H-111, SPEC-015 §2.6.4)", () => {
  const curve: ResponseCurveDto = {
    freqs_hz: [20, 100, 1_000, 10_000, 20_000],
    sample_rate_hz: 48_000,
    total_db: [0, 6, -6, 3, 0],
    components_db: [],
  };

  it("returns the exact value at a known point", () => {
    expect(totalDbAtFreq(curve, 1_000)).toBe(-6);
    expect(totalDbAtFreq(curve, 20)).toBe(0);
    expect(totalDbAtFreq(curve, 20_000)).toBe(0);
  });

  it("linearly interpolates between two bracketing points", () => {
    // Halfway between 100 (6 dB) and 1000 (-6 dB) in curve-index space -> 0 dB.
    expect(totalDbAtFreq(curve, 550)).toBeCloseTo(0, 5);
    // A quarter of the way from 1000 (-6 dB) to 10000 (3 dB) -> -6 + 0.25 * 9 = -3.75 dB.
    expect(totalDbAtFreq(curve, 3_250)).toBeCloseTo(-3.75, 5);
  });

  it("clamps to the edge value outside the curve's own range (no extrapolation)", () => {
    expect(totalDbAtFreq(curve, 10)).toBe(0);
    expect(totalDbAtFreq(curve, 24_000)).toBe(0);
  });

  it("is null for an empty curve", () => {
    const empty: ResponseCurveDto = { freqs_hz: [], sample_rate_hz: 48_000, total_db: [], components_db: [] };
    expect(totalDbAtFreq(empty, 1_000)).toBeNull();
  });

  it("returns the single point for a one-point curve", () => {
    const one: ResponseCurveDto = { freqs_hz: [1_000], sample_rate_hz: 48_000, total_db: [4], components_db: [] };
    expect(totalDbAtFreq(one, 20)).toBe(4);
    expect(totalDbAtFreq(one, 20_000)).toBe(4);
  });
});

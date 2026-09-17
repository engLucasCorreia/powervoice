import { describe, expect, it } from "vitest";
import { GR_SCALE_FLOOR_DB, grAtFloor, grFraction, grScaleMin, grTicks } from "./grMeter";

/** SPEC-016 §2.6 "Gain-reduction meters"; SPEC-017 §2.3 keeps the limiter's narrower scale. */
describe("gain-reduction meter scale", () => {
  it("runs 0 … −30 dB, and keeps a channel's own narrower range", () => {
    expect(grScaleMin(-60)).toBe(GR_SCALE_FLOOR_DB);
    expect(grScaleMin(-24)).toBe(-24);
    expect(grScaleMin(Number.NEGATIVE_INFINITY)).toBe(GR_SCALE_FLOOR_DB);
  });

  it("puts −6 dB a fifth along the −30 dB scale and pins anything past it", () => {
    expect(grFraction(0, -30)).toBe(0);
    expect(grFraction(-6, -30)).toBeCloseTo(0.2, 12);
    expect(grFraction(-30, -30)).toBe(1);
    expect(grFraction(-45, -30)).toBe(1);
    expect(grFraction(3, -30)).toBe(0);
  });

  it("ticks at 0, −3, −6, −10, −20 and −30, dropping any outside the scale", () => {
    expect(grTicks(-30).map((t) => t.db)).toEqual([0, -3, -6, -10, -20, -30]);
    expect(grTicks(-24).map((t) => t.db)).toEqual([0, -3, -6, -10, -20]);
  });

  it("knows when the value has reached the channel floor", () => {
    expect(grAtFloor(-60, -60)).toBe(true);
    expect(grAtFloor(-59.9, -60)).toBe(false);
  });
});

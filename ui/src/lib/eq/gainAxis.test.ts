import { describe, expect, it } from "vitest";
import {
  EQ_GAIN_RANGE_DEFAULT_DB,
  EQ_GAIN_RANGE_WIDE_DB,
  dbForY,
  formatGainDb,
  gainAxisTicks,
  gainGridStepDb,
  yForDb,
} from "./gainAxis";

/** Linear-dB gain axis tests (S3-07, SPEC-015 §2.6.2). */

describe("yForDb / dbForY round trip", () => {
  const height = 160;
  const range = EQ_GAIN_RANGE_DEFAULT_DB;

  it("0 dB sits at mid-height", () => {
    expect(yForDb(0, height, range)).toBeCloseTo(height / 2, 6);
  });

  it("+range dB is the top edge, -range dB the bottom edge", () => {
    expect(yForDb(range, height, range)).toBeCloseTo(0, 6);
    expect(yForDb(-range, height, range)).toBeCloseTo(height, 6);
  });

  it("round-trips exactly", () => {
    for (const db of [-11.9, -6, -0.5, 0, 3.2, 12]) {
      const y = yForDb(db, height, range);
      expect(dbForY(y, height, range)).toBeCloseTo(db, 6);
    }
  });

  it("clamps beyond the range", () => {
    expect(yForDb(100, height, range)).toBe(0);
    expect(yForDb(-100, height, range)).toBe(height);
    expect(dbForY(-50, height, range)).toBe(range);
    expect(dbForY(height + 50, height, range)).toBe(-range);
  });

  it("the ±24 dB range halves a given gain's screen displacement from centre", () => {
    const y12 = yForDb(6, height, 12);
    const y24 = yForDb(6, height, 24);
    expect(height / 2 - y24).toBeCloseTo((height / 2 - y12) / 2, 6);
  });
});

describe("gainGridStepDb (H-24 item 8)", () => {
  it("is 3 dB for the default ±12 dB range, 6 dB for the wide ±24 dB one", () => {
    expect(gainGridStepDb(EQ_GAIN_RANGE_DEFAULT_DB)).toBe(3);
    expect(gainGridStepDb(EQ_GAIN_RANGE_WIDE_DB)).toBe(6);
  });
});

describe("formatGainDb (H-24 item 8)", () => {
  it("signs positive values, leaves 0 and negative values as-is", () => {
    expect(formatGainDb(12)).toBe("+12");
    expect(formatGainDb(0)).toBe("0");
    expect(formatGainDb(-12)).toBe("-12");
  });
});

describe("gainAxisTicks (H-24 item 8: dB labels at ±12/±24)", () => {
  it("spans -range..range at the grid step, 0 dB at mid-height", () => {
    const ticks = gainAxisTicks(160, EQ_GAIN_RANGE_DEFAULT_DB);
    expect(ticks.map((t) => t.db)).toEqual([-12, -9, -6, -3, 0, 3, 6, 9, 12]);
    const zero = ticks.find((t) => t.db === 0)!;
    expect(zero.y).toBeCloseTo(80, 6);
    expect(zero.label).toBe("0");
  });

  it("uses the 6 dB step for the wide range", () => {
    const ticks = gainAxisTicks(160, EQ_GAIN_RANGE_WIDE_DB);
    expect(ticks.map((t) => t.db)).toEqual([-24, -18, -12, -6, 0, 6, 12, 18, 24]);
  });

  it("is empty for a non-positive height or range", () => {
    expect(gainAxisTicks(0, EQ_GAIN_RANGE_DEFAULT_DB)).toEqual([]);
    expect(gainAxisTicks(160, 0)).toEqual([]);
  });
});

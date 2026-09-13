import { describe, expect, it } from "vitest";
import { EQ_GAIN_RANGE_DEFAULT_DB, dbForY, yForDb } from "./gainAxis";

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

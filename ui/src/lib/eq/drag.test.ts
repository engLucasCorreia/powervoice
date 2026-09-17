import { describe, expect, it } from "vitest";
import { dragPosition, wheelNotches, wheelQFactor } from "./drag";

/** Node drag/wheel pixel math tests (S3-07 + H-86, SPEC-015 §2.6.4). */

describe("dragPosition", () => {
  it("adds the raw pointer delta when not fine", () => {
    const pos = dragPosition({ freqPx: 100, gainPx: 50 }, 20, -10, false);
    expect(pos).toEqual({ freqPx: 120, gainPx: 40 });
  });

  it("scales the delta by 0.1 when fine (Shift)", () => {
    const pos = dragPosition({ freqPx: 100, gainPx: 50 }, 20, -10, true);
    expect(pos.freqPx).toBeCloseTo(102, 6);
    expect(pos.gainPx).toBeCloseTo(49, 6);
  });

  it("keeps gainPx null for HP/LP (horizontal movement only)", () => {
    const pos = dragPosition({ freqPx: 0, gainPx: null }, 5, 5, false);
    expect(pos.gainPx).toBeNull();
    expect(pos.freqPx).toBe(5);
  });
});

describe("wheelQFactor", () => {
  it("is 2^(1/6) per notch up, 2^(-1/6) down", () => {
    expect(wheelQFactor(-1, false)).toBeCloseTo(2 ** (1 / 6), 9);
    expect(wheelQFactor(1, false)).toBeCloseTo(2 ** (-1 / 6), 9);
  });

  it("is 2^(1/24) per notch when fine (Shift)", () => {
    expect(wheelQFactor(-1, true)).toBeCloseTo(2 ** (1 / 24), 9);
    expect(wheelQFactor(1, true)).toBeCloseTo(2 ** (-1 / 24), 9);
  });

  it("is 1 for a zero delta", () => {
    expect(wheelQFactor(0, false)).toBe(1);
  });
});

describe("wheelNotches", () => {
  it("is 1 up, -1 down, 0 for no movement", () => {
    expect(wheelNotches(-1)).toBe(1);
    expect(wheelNotches(1)).toBe(-1);
    expect(wheelNotches(0)).toBe(0);
  });
});

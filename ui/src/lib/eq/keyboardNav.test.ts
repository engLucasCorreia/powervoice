import { describe, expect, it } from "vitest";
import {
  EQ_KEY_GAIN_STEP_DB,
  EQ_KEY_GAIN_STEP_FINE_DB,
  keyFreqFactor,
  keyFreqHz,
  keyGainDb,
  keyQFactor,
  keyQValue,
  stepSlopeIndex,
} from "./keyboardNav";

/** AC-19 (SPEC-015 §2.6.5): the exact per-key step table. */
describe("keyFreqFactor / keyFreqHz (← / →)", () => {
  it("is 2^(1/12) per key, 2^(-1/12) the other way", () => {
    expect(keyFreqFactor(1, false)).toBeCloseTo(2 ** (1 / 12), 12);
    expect(keyFreqFactor(-1, false)).toBeCloseTo(2 ** (-1 / 12), 12);
  });

  it("Shift is the fine 2^(1/48) step", () => {
    expect(keyFreqFactor(1, true)).toBeCloseTo(2 ** (1 / 48), 12);
    expect(keyFreqFactor(-1, true)).toBeCloseTo(2 ** (-1 / 48), 12);
  });

  it("multiplies the current frequency and clamps to the param range", () => {
    expect(keyFreqHz(1_000, 1, false, 20, 20_000)).toBeCloseTo(1_000 * 2 ** (1 / 12), 6);
    expect(keyFreqHz(19_999, 1, false, 20, 20_000)).toBe(20_000);
    expect(keyFreqHz(20.1, -1, false, 20, 20_000)).toBe(20);
  });
});

describe("keyGainDb (↑ / ↓)", () => {
  it("is ±0.5 dB, Shift ±0.1 dB", () => {
    expect(keyGainDb(0, 1, false, -24, 24)).toBeCloseTo(EQ_KEY_GAIN_STEP_DB, 9);
    expect(keyGainDb(0, -1, false, -24, 24)).toBeCloseTo(-EQ_KEY_GAIN_STEP_DB, 9);
    expect(keyGainDb(0, 1, true, -24, 24)).toBeCloseTo(EQ_KEY_GAIN_STEP_FINE_DB, 9);
    expect(keyGainDb(0, -1, true, -24, 24)).toBeCloseTo(-EQ_KEY_GAIN_STEP_FINE_DB, 9);
  });

  it("clamps to the param range", () => {
    expect(keyGainDb(23.9, 1, false, -24, 24)).toBe(24);
    expect(keyGainDb(-23.9, -1, false, -24, 24)).toBe(-24);
  });
});

describe("keyQFactor / keyQValue (PageUp / PageDown)", () => {
  it("is the same 2^(1/6) coarse factor as the wheel's", () => {
    expect(keyQFactor(1)).toBeCloseTo(2 ** (1 / 6), 12);
    expect(keyQFactor(-1)).toBeCloseTo(2 ** (-1 / 6), 12);
  });

  it("multiplies the current Q and clamps to the param range", () => {
    expect(keyQValue(1, 1, 0.1, 30)).toBeCloseTo(2 ** (1 / 6), 9);
    expect(keyQValue(29, 1, 0.1, 30)).toBe(30);
    expect(keyQValue(0.11, -1, 0.1, 30)).toBe(0.1);
  });
});

describe("stepSlopeIndex (HP/LP ↑ / ↓)", () => {
  it("moves one enum step and clamps to [0, maxIndex]", () => {
    expect(stepSlopeIndex(3, 1, 7)).toBe(4);
    expect(stepSlopeIndex(3, -1, 7)).toBe(2);
    expect(stepSlopeIndex(7, 1, 7)).toBe(7);
    expect(stepSlopeIndex(0, -1, 7)).toBe(0);
  });
});

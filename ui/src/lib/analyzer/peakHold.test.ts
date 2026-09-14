import { describe, expect, it } from "vitest";
import {
  PEAK_FALL_DB_PER_S,
  PEAK_HOLD_S,
  createPeakHold,
  resetPeakHold,
  updatePeakHold,
} from "./peakHold";

const RATE_HZ = 60;
const DT = 1 / RATE_HZ;

describe("peak-hold ballistics (SPEC-007 AC-17)", () => {
  it("holds a peak for 2.0 s ± 1 frame, then falls at 12.0 dB/s", () => {
    let state = createPeakHold(1);
    // One loud frame, then silence.
    state = updatePeakHold(state, [-10], DT);
    expect(state[0]!.value).toBe(-10);

    const framesInHold = Math.round(PEAK_HOLD_S * RATE_HZ);
    for (let i = 0; i < framesInHold - 1; i++) {
      state = updatePeakHold(state, [-100], DT);
      expect(state[0]!.value).toBeCloseTo(-10, 6);
    }

    // One frame past the nominal hold, ballistics may have just started falling (± 1 frame).
    state = updatePeakHold(state, [-100], DT);
    expect(state[0]!.value).toBeLessThanOrEqual(-10);
    expect(state[0]!.value).toBeGreaterThan(-10 - PEAK_FALL_DB_PER_S * (2 * DT));

    // Run one more second: should have fallen close to 12 dB/s * 1 s.
    let before = state[0]!.value;
    for (let i = 0; i < RATE_HZ; i++) {
      state = updatePeakHold(state, [-100], DT);
    }
    const fell = before - state[0]!.value;
    expect(fell).toBeCloseTo(PEAK_FALL_DB_PER_S, 0);
  });

  it("re-arms the hold when the live level catches up during the fall", () => {
    let state = createPeakHold(1);
    state = updatePeakHold(state, [-10], DT);
    for (let i = 0; i < Math.round(PEAK_HOLD_S * RATE_HZ) + 30; i++) {
      state = updatePeakHold(state, [-100], DT);
    }
    expect(state[0]!.value).toBeLessThan(-10);
    state = updatePeakHold(state, [-5], DT);
    expect(state[0]!.value).toBe(-5);
    expect(state[0]!.holdRemainingS).toBeCloseTo(PEAK_HOLD_S, 6);
  });

  it("never falls below the current instantaneous level", () => {
    let state = createPeakHold(1);
    state = updatePeakHold(state, [-10], DT);
    for (let i = 0; i < 1000; i++) {
      state = updatePeakHold(state, [-40], DT);
    }
    expect(state[0]!.value).toBeCloseTo(-40, 6);
  });

  it("a click or a RESET frame clears the hold", () => {
    let state = createPeakHold(2);
    state = updatePeakHold(state, [-10, -5], DT);
    expect(state[0]!.value).toBe(-10);
    expect(state[1]!.value).toBe(-5);
    resetPeakHold(state);
    expect(state.every((b) => b.value === Number.NEGATIVE_INFINITY)).toBe(true);
    expect(state.every((b) => b.holdRemainingS === 0)).toBe(true);
  });
});

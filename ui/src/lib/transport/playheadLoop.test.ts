import { describe, expect, it } from "vitest";
import { PlayheadExtrapolator, extrapolate, wrapInLoop } from "./playhead";

/** H-37 (SPEC-003 §2.2): the displayed playhead wraps inside the loop range while looping. */
describe("wrapInLoop", () => {
  const loop = [1_000, 2_000] as const;

  it("wraps a position past the loop end back from the loop start", () => {
    expect(wrapInLoop(1_500, 2_000, loop)).toBe(1_000);
    expect(wrapInLoop(1_500, 2_250, loop)).toBe(1_250);
    expect(wrapInLoop(1_500, 4_100, loop)).toBe(1_100); // two passes on
  });

  it("leaves positions before the loop end, past-the-loop anchors and no loop alone", () => {
    expect(wrapInLoop(1_500, 1_999, loop)).toBe(1_999);
    expect(wrapInLoop(500, 900, loop)).toBe(900); // heading into the loop
    expect(wrapInLoop(500, 2_100, loop)).toBe(1_100); // ...and through it
    expect(wrapInLoop(2_500, 3_000, loop)).toBe(3_000); // playing past the loop: not looping
    expect(wrapInLoop(1_500, 2_500, null)).toBe(2_500);
  });
});

describe("extrapolation across a loop wrap", () => {
  const rate = 48_000;

  it("extrapolate wraps; PlayheadExtrapolator follows its loop range", () => {
    const anchor = { sample: 1_900, timeNs: 0, rate };
    // 200 samples later (4.1667 ms): 2_100 → 1_100 within [1_000, 2_000).
    const t = (200 / rate) * 1e9;
    expect(extrapolate(anchor, t, 10_000, [1_000, 2_000])).toBeCloseTo(1_100, 6);
    expect(extrapolate(anchor, t, 10_000)).toBeCloseTo(2_100, 6);

    const x = new PlayheadExtrapolator();
    x.setLoop([1_000, 2_000]);
    x.update(anchor, 0, 10_000);
    expect(x.position(t, 10_000)).toBeCloseTo(1_100, 6);
    // The post-wrap anchor agrees with the wrapped prediction: no jump, no slew offset.
    x.update({ sample: 1_100, timeNs: t, rate }, t, 10_000);
    expect(x.position(t, 10_000)).toBeCloseTo(1_100, 6);
    x.setLoop(null);
    expect(x.position(t * 2, 10_000)).toBeCloseTo(1_300, 6);
  });
});

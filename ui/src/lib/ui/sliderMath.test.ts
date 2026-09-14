import { describe, expect, it } from "vitest";
import { fractionOf, keyToValue, snapToStep, valueAtPosition } from "./slider";

describe("snapToStep", () => {
  it("clamps into range and snaps to the step grid anchored at min", () => {
    expect(snapToStep(5.3, -60, 12, 0.5)).toBe(5.5);
    expect(snapToStep(-100, -60, 12, 0.5)).toBe(-60);
    expect(snapToStep(99, -60, 12, 0.5)).toBe(12);
    expect(snapToStep(0.1 + 0.2, 0, 1, 0.1)).toBe(0.3);
  });

  it("leaves the value alone (clamped) with step 0", () => {
    expect(snapToStep(0.123, 0, 1, 0)).toBe(0.123);
  });
});

describe("fractionOf", () => {
  it("maps value to 0..1 and guards a zero-width range", () => {
    expect(fractionOf(0, -12, 12)).toBe(0.5);
    expect(fractionOf(-20, -12, 12)).toBe(0);
    expect(fractionOf(5, 5, 5)).toBe(0);
  });
});

describe("valueAtPosition", () => {
  it("maps a pointer x within the track to a snapped value", () => {
    expect(valueAtPosition(50, 100, 0, 10, 1)).toBe(5);
    expect(valueAtPosition(-10, 100, 0, 10, 1)).toBe(0);
    expect(valueAtPosition(250, 100, 0, 10, 1)).toBe(10);
  });

  it("returns min for a zero-width track (jsdom)", () => {
    expect(valueAtPosition(10, 0, -5, 5, 1)).toBe(-5);
  });
});

describe("keyToValue", () => {
  const range = { min: 0, max: 100, step: 1, bigStep: 10 };

  it("arrows move one step, Shift+arrows and PageUp/Down move a big step", () => {
    expect(keyToValue("ArrowRight", false, 50, range)).toBe(51);
    expect(keyToValue("ArrowUp", false, 50, range)).toBe(51);
    expect(keyToValue("ArrowLeft", false, 50, range)).toBe(49);
    expect(keyToValue("ArrowDown", true, 50, range)).toBe(40);
    expect(keyToValue("PageUp", false, 50, range)).toBe(60);
    expect(keyToValue("PageDown", false, 50, range)).toBe(40);
  });

  it("Home/End jump to the ends and results stay clamped", () => {
    expect(keyToValue("Home", false, 50, range)).toBe(0);
    expect(keyToValue("End", false, 50, range)).toBe(100);
    expect(keyToValue("ArrowRight", true, 95, range)).toBe(100);
  });

  it("returns null for other keys", () => {
    expect(keyToValue("Enter", false, 50, range)).toBeNull();
  });
});

import { describe, expect, it } from "vitest";
import { nextRovingIndex } from "./roving";

const none = [false, false, false, false];

describe("nextRovingIndex (arrow-key navigation for tabs/segments)", () => {
  it("moves with arrows and wraps around", () => {
    expect(nextRovingIndex(0, "ArrowRight", none)).toBe(1);
    expect(nextRovingIndex(3, "ArrowRight", none)).toBe(0);
    expect(nextRovingIndex(0, "ArrowLeft", none)).toBe(3);
    expect(nextRovingIndex(2, "ArrowDown", none)).toBe(3);
    expect(nextRovingIndex(2, "ArrowUp", none)).toBe(1);
  });

  it("jumps with Home/End", () => {
    expect(nextRovingIndex(2, "Home", none)).toBe(0);
    expect(nextRovingIndex(1, "End", none)).toBe(3);
  });

  it("skips disabled items", () => {
    const disabled = [false, true, false, true];
    expect(nextRovingIndex(0, "ArrowRight", disabled)).toBe(2);
    expect(nextRovingIndex(2, "ArrowRight", disabled)).toBe(0);
    expect(nextRovingIndex(0, "ArrowLeft", disabled)).toBe(2);
    expect(nextRovingIndex(2, "End", disabled)).toBe(2);
    expect(nextRovingIndex(2, "Home", [true, false, false])).toBe(1);
  });

  it("returns null for keys it doesn't handle or when everything is disabled", () => {
    expect(nextRovingIndex(0, "Enter", none)).toBeNull();
    expect(nextRovingIndex(0, "ArrowRight", [true, true])).toBeNull();
  });

  it("respects orientation: horizontal ignores Up/Down, vertical ignores Left/Right", () => {
    expect(nextRovingIndex(0, "ArrowDown", none, "horizontal")).toBeNull();
    expect(nextRovingIndex(0, "ArrowRight", none, "vertical")).toBeNull();
    expect(nextRovingIndex(0, "ArrowDown", none, "vertical")).toBe(1);
  });
});

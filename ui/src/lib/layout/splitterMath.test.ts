import { describe, expect, it } from "vitest";
import {
  clampColumnWidthPx,
  clampDockHeightPx,
  clampSize,
  resizeColumnPx,
  stepSizePx,
} from "./splitterMath";

describe("clampSize", () => {
  it("passes values already inside the range through unchanged", () => {
    expect(clampSize(50, 0, 100)).toBe(50);
  });

  it("clamps below the minimum and above the maximum", () => {
    expect(clampSize(-10, 0, 100)).toBe(0);
    expect(clampSize(200, 0, 100)).toBe(100);
  });

  it("tolerates a swapped min/max", () => {
    expect(clampSize(50, 100, 0)).toBe(50);
    expect(clampSize(-5, 100, 0)).toBe(0);
  });
});

describe("resizeColumnPx (H-24 item 2: draggable vertical splitters)", () => {
  it("grows by the drag delta for a left-edge (non-reversed) splitter", () => {
    expect(resizeColumnPx(240, 30, false, 160, 480)).toBe(270);
  });

  it("shrinks by a negative delta", () => {
    expect(resizeColumnPx(240, -30, false, 160, 480)).toBe(210);
  });

  it("reverses the delta's sign for a right-edge splitter (the Rack column)", () => {
    // Dragging left (negative deltaPx) grows a column whose splitter is on its left edge.
    expect(resizeColumnPx(280, -40, true, 160, 480)).toBe(320);
    expect(resizeColumnPx(280, 40, true, 160, 480)).toBe(240);
  });

  it("clamps to the min/max width", () => {
    expect(resizeColumnPx(240, -1000, false, 160, 480)).toBe(160);
    expect(resizeColumnPx(240, 1000, false, 160, 480)).toBe(480);
  });
});

describe("stepSizePx (H-24: keyboard-accessible splitters, arrow keys)", () => {
  it("grows or shrinks by exactly one step", () => {
    expect(stepSizePx(240, 1, 160, 480, 16)).toBe(256);
    expect(stepSizePx(240, -1, 160, 480, 16)).toBe(224);
  });

  it("clamps at the bounds", () => {
    expect(stepSizePx(160, -1, 160, 480, 16)).toBe(160);
    expect(stepSizePx(480, 1, 160, 480, 16)).toBe(480);
  });

  it("defaults to the SPEC step size when omitted", () => {
    expect(stepSizePx(240, 1, 0, 1000)).toBe(256);
  });
});

describe("clampDockHeightPx (H-24 item 1: dock [120, 60%], workspace floor 40%)", () => {
  it("passes a value already inside the range through", () => {
    expect(clampDockHeightPx(1000, 240)).toBe(240);
  });

  it("clamps to the 60% ceiling, guaranteeing the workspace's 40% floor", () => {
    expect(clampDockHeightPx(1000, 900)).toBe(600);
  });

  it("clamps to the 120px floor", () => {
    expect(clampDockHeightPx(1000, 10)).toBe(120);
  });

  it("degrades the floor for a very short window instead of exceeding 60%", () => {
    // 60% of 150 is 90, below the usual 120px floor — the floor must yield, not the ceiling.
    expect(clampDockHeightPx(150, 500)).toBe(90);
    expect(clampDockHeightPx(150, 0)).toBe(90);
  });

  it("never returns negative or NaN for a zero/negative total height", () => {
    expect(clampDockHeightPx(0, 240)).toBe(240);
    expect(clampDockHeightPx(-5, 240)).toBe(240);
  });
});

describe("clampColumnWidthPx", () => {
  it("clamps to [min, available]", () => {
    expect(clampColumnWidthPx(500, 160, 400)).toBe(400);
    expect(clampColumnWidthPx(50, 160, 400)).toBe(160);
    expect(clampColumnWidthPx(200, 160, 400)).toBe(200);
  });

  it("never lets the max collapse below the min", () => {
    expect(clampColumnWidthPx(200, 160, 100)).toBe(160);
  });
});

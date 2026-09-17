import { describe, expect, it } from "vitest";
import type { TransferCurveFrame } from "../ipc/transferCurve";
import {
  branchToScreen,
  componentIsActive,
  componentToScreen,
  differingRanges,
  sliceScreen,
} from "./curvePoints";
import { xForLevel, yForLevel } from "./levelAxis";

const SIDE = 300;
const MUTED = Number.NEGATIVE_INFINITY;

function curve(inDbfs: number[], rising: number[], falling?: number[]): TransferCurveFrame {
  return {
    seq: 1,
    inDbfs,
    rising,
    falling: falling ?? null,
    components: [],
    handles: [],
  };
}

describe("branchToScreen", () => {
  it("maps each level pair onto the axes", () => {
    const c = curve([-80, -40, 0], [-80, -40, -6]);
    const points = branchToScreen(c, c.rising, SIDE, SIDE);
    expect(points).toHaveLength(3);
    expect(points[1]).toEqual({ x: xForLevel(-40, SIDE), y: yForLevel(-40, SIDE) });
    expect(points[2]).toEqual({ x: xForLevel(0, SIDE), y: yForLevel(-6, SIDE) });
  });

  it("breaks the line where the module mutes (`VXTC` carries Rust's −inf)", () => {
    const c = curve([-80, -60, -40], [MUTED, MUTED, -40]);
    const points = branchToScreen(c, c.rising, SIDE, SIDE);
    expect(points[0]).toBeNull();
    expect(points[1]).toBeNull();
    expect(points[2]).not.toBeNull();
  });

  it("drops non-finite values instead of handing them to the canvas", () => {
    const c = curve([-80, Number.NaN, -40], [-80, -70, Number.POSITIVE_INFINITY]);
    const points = branchToScreen(c, c.rising, SIDE, SIDE);
    expect(points[1]).toBeNull();
    expect(points[2]).toBeNull();
  });

  it("draws a level below the graph floor at the floor (only −inf breaks the line)", () => {
    const c = curve([-80], [-140]);
    expect(branchToScreen(c, c.rising, SIDE, SIDE)[0]).toEqual({
      x: xForLevel(-80, SIDE),
      y: yForLevel(-140, SIDE),
    });
  });
});

describe("componentToScreen (H-77: per-component overlays)", () => {
  it("plots input + the section's own gain", () => {
    const c = curve([-40, 0], [-40, -6]);
    const points = componentToScreen(c, [0, -6], SIDE, SIDE);
    expect(points[0]).toEqual({ x: xForLevel(-40, SIDE), y: yForLevel(-40, SIDE) });
    expect(points[1]).toEqual({ x: xForLevel(0, SIDE), y: yForLevel(-6, SIDE) });
  });

  it("breaks where the section mutes", () => {
    const c = curve([-80, -40], [MUTED, -40]);
    expect(componentToScreen(c, [MUTED, 0], SIDE, SIDE)[0]).toBeNull();
  });

  it("only an active section is worth drawing", () => {
    expect(componentIsActive([0, 0, 0])).toBe(false);
    expect(componentIsActive([0, 0, -3])).toBe(true);
    expect(componentIsActive([MUTED, 0])).toBe(true);
  });
});

describe("differingRanges (the hysteresis loop)", () => {
  it("is empty when the branches agree", () => {
    expect(differingRanges([-80, -40, 0], [-80, -40, 0])).toEqual([]);
  });

  it("covers only where they differ, widened by one sample so it meets the solid curve", () => {
    const rising = [-80, -80, -80, -40, -30];
    const falling = [-80, -70, -60, -40, -30];
    expect(differingRanges(rising, falling)).toEqual([[0, 3]]);
  });

  it("closes a range that runs to the end", () => {
    expect(differingRanges([0, 0, 0], [0, -6, -12])).toEqual([[0, 2]]);
  });

  it("matches −inf against −inf as equal, not differing", () => {
    const muted = Number.NEGATIVE_INFINITY;
    expect(differingRanges([muted, -40], [muted, -40])).toEqual([]);
    expect(differingRanges([muted, -40], [-70, -40])).toEqual([[0, 1]]);
  });

  it("sliceScreen returns the inclusive sub-path", () => {
    const points = [{ x: 0, y: 0 }, { x: 1, y: 1 }, { x: 2, y: 2 }, { x: 3, y: 3 }];
    expect(sliceScreen(points, [1, 2])).toEqual([{ x: 1, y: 1 }, { x: 2, y: 2 }]);
  });
});

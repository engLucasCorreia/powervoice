import { describe, expect, it } from "vitest";
import {
  TRANSFER_MAX_DBFS,
  TRANSFER_MAX_POINTS,
  TRANSFER_MAX_SIDE_PX,
  TRANSFER_MIN_DBFS,
  TRANSFER_MIN_SIDE_PX,
  levelForX,
  transferAxisTicks,
  transferCurvePointCount,
  transferSidePx,
  xForLevel,
  yForLevel,
} from "./levelAxis";

/** H-63 (SPEC-016 §2.6): the transfer graph's shared dBFS axis. */
describe("level axis", () => {
  it("maps the range ends to the canvas edges and back", () => {
    expect(xForLevel(TRANSFER_MIN_DBFS, 300)).toBe(0);
    expect(xForLevel(TRANSFER_MAX_DBFS, 300)).toBe(300);
    expect(yForLevel(TRANSFER_MIN_DBFS, 300)).toBe(300);
    expect(yForLevel(TRANSFER_MAX_DBFS, 300)).toBe(0);
    for (const db of [-80, -60, -37, -12, 0, 6]) {
      expect(levelForX(xForLevel(db, 300), 300)).toBeCloseTo(db, 9);
    }
  });

  it("clamps out-of-range levels instead of drawing off the canvas", () => {
    expect(xForLevel(-200, 300)).toBe(0);
    expect(xForLevel(24, 300)).toBe(300);
    expect(levelForX(-50, 300)).toBe(TRANSFER_MIN_DBFS);
    expect(levelForX(999, 300)).toBe(TRANSFER_MAX_DBFS);
    expect(levelForX(10, 0)).toBe(TRANSFER_MIN_DBFS);
  });

  it("uses the same scale on both axes, so 1:1 is the square's diagonal", () => {
    for (const db of [-72, -24, 0, 6]) {
      expect(xForLevel(db, 260)).toBeCloseTo(260 - yForLevel(db, 260), 9);
    }
  });

  it("grids every 6 dB and labels every 12 (SPEC-016 §2.6)", () => {
    const ticks = transferAxisTicks();
    expect(ticks[0]?.db).toBe(-78);
    expect(ticks.at(-1)?.db).toBe(6);
    for (let i = 1; i < ticks.length; i += 1) {
      expect((ticks[i]!.db - ticks[i - 1]!.db)).toBe(6);
    }
    expect(ticks.filter((t) => t.major).map((t) => t.db)).toEqual([
      -72, -60, -48, -36, -24, -12, 0,
    ]);
    // H-26: labels use the true minus sign, and positives are signed.
    expect(ticks.find((t) => t.db === -12)?.label).toBe("−12");
    expect(ticks.find((t) => t.db === 6)?.label).toBe("+6");
  });

  it("keeps the graph square within the spec's 200–320 px", () => {
    expect(transferSidePx(0)).toBe(TRANSFER_MIN_SIDE_PX);
    expect(transferSidePx(120)).toBe(TRANSFER_MIN_SIDE_PX);
    expect(transferSidePx(260)).toBe(260);
    expect(transferSidePx(900)).toBe(TRANSFER_MAX_SIDE_PX);
  });

  it("asks for one point per pixel column, within the backend's cap", () => {
    expect(transferCurvePointCount(0)).toBe(2);
    expect(transferCurvePointCount(260)).toBe(260);
    expect(transferCurvePointCount(99_999)).toBe(TRANSFER_MAX_POINTS);
  });
});

import { describe, expect, it } from "vitest";
import { amplitudeTicksDbfs, centerlineY, yForAmplitudeDb } from "./amplitudeAxis";

describe("yForAmplitudeDb", () => {
  it("puts 0 dBFS at the top edge and the centerline at -Infinity dB", () => {
    expect(yForAmplitudeDb(0, 1, 200)).toBe(0);
    expect(yForAmplitudeDb(-Infinity, 1, 200)).toBe(100);
  });

  it("halves the distance from center every -6.02 dB (within float rounding)", () => {
    const y6 = yForAmplitudeDb(-6.020599913279624, 1, 200); // exactly -6.0206 dB = amplitude 0.5
    expect(y6).toBeCloseTo(50, 5);
  });

  it("verticalZoom scales the amplitude before mapping to y", () => {
    // At 2x zoom, 0 dBFS' amplitude (1.0) maps to 2.0 -> clamped off-screen above the top by the
    // raw formula (no clamping here — that's the caller's job when filtering visible ticks).
    expect(yForAmplitudeDb(0, 2, 200)).toBe(-100);
  });
});

describe("centerlineY", () => {
  it("is exactly half the height", () => {
    expect(centerlineY(200)).toBe(100);
    expect(centerlineY(201)).toBe(100.5);
  });
});

describe("amplitudeTicksDbfs (H-24 item 7, SPEC-006 §2.4/§4.2)", () => {
  it("includes 0 dBFS at both edges and never labels the centerline", () => {
    const ticks = amplitudeTicksDbfs(400, 1, 10);
    const zero = ticks.filter((t) => t.db === 0);
    expect(zero).toHaveLength(2);
    expect(zero.map((t) => t.y).sort((a, b) => a - b)).toEqual([0, 400]);
    expect(ticks.some((t) => t.y === 200)).toBe(false); // the centerline itself
  });

  it("is symmetric: every tick's dB value appears twice, mirrored around the centerline", () => {
    const ticks = amplitudeTicksDbfs(1000, 1, 5);
    const byDb = new Map<number, number[]>();
    for (const t of ticks) {
      byDb.set(t.db, [...(byDb.get(t.db) ?? []), t.y]);
    }
    for (const [db, ys] of byDb) {
      expect(ys, `db=${db}`).toHaveLength(2);
      const [a, b] = ys.sort((x, y) => x - y);
      expect(a! + b!).toBeCloseTo(1000, 5); // mirrored around centerY = 500
    }
  });

  it("thins by the minimum pixel gap, keeping the loudest (0, -6, -12 ...) ticks first", () => {
    const ticks = amplitudeTicksDbfs(60, 1, 100); // tiny pane: almost everything collides
    // Only the very edges (0 dBFS top/bottom) can possibly survive a 100px minimum gap on a 60px pane.
    expect(ticks.every((t) => t.db === 0)).toBe(true);
  });

  it("returns ticks in increasing y order", () => {
    const ticks = amplitudeTicksDbfs(400, 1, 10);
    for (let i = 1; i < ticks.length; i++) {
      expect(ticks[i]!.y).toBeGreaterThanOrEqual(ticks[i - 1]!.y);
    }
  });

  it("drops ticks beyond the visible amplitude at a given verticalZoom", () => {
    // At 4x zoom, only amplitudes <= 0.25 (-12.04 dBFS and below) fit within +-1.0, so the ladder's
    // -12 entry (amplitude 0.251) is just barely excluded and -18 is the first one that survives.
    const ticks = amplitudeTicksDbfs(400, 4, 1);
    expect(ticks.some((t) => t.db === 0)).toBe(false);
    expect(ticks.some((t) => t.db === -12)).toBe(false);
    expect(ticks.some((t) => t.db === -18)).toBe(true);
  });

  it("returns an empty list for a non-positive height", () => {
    expect(amplitudeTicksDbfs(0, 1, 10)).toEqual([]);
    expect(amplitudeTicksDbfs(-10, 1, 10)).toEqual([]);
  });
});

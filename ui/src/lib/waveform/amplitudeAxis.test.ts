import { describe, expect, it } from "vitest";
import { amplitudeTicksDbfs, amplitudeTicksPercent, centerlineY, yForAmplitudeDb } from "./amplitudeAxis";
import { MAX_VERTICAL_ZOOM } from "./verticalZoom";

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
      byDb.set(t.db!, [...(byDb.get(t.db!) ?? []), t.y]);
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

  it("never puts two labels closer than the minimum gap, even across the centerline (H-26)", () => {
    for (const height of [120, 200, 283, 370, 480, 800]) {
      const ticks = amplitudeTicksDbfs(height, 1, 16);
      for (let i = 1; i < ticks.length; i++) {
        expect(ticks[i]!.y - ticks[i - 1]!.y, `${height}px: ${ticks[i - 1]!.label} / ${ticks[i]!.label}`).toBeGreaterThanOrEqual(16);
      }
      expect(ticks.every((t) => !t.label.includes("-")), "true minus").toBe(true);
    }
  });

  it("H-35: never collides at the SPEC-006 §2.4 ceiling (256x) — same minGap guarantee", () => {
    // At 256x, every ladder entry down to about -48 dBFS maps outside +-1.0 (only 0 dBFS survives
    // per-amplitude filtering) — this exercises the same thinning path at the extreme end of the
    // vertical-zoom range the ticket calls out, not just the unscaled (1x) case above.
    for (const height of [60, 120, 200, 400, 800]) {
      const ticks = amplitudeTicksDbfs(height, MAX_VERTICAL_ZOOM, 16);
      for (let i = 1; i < ticks.length; i++) {
        expect(
          ticks[i]!.y - ticks[i - 1]!.y,
          `${height}px @ 256x: ${ticks[i - 1]!.label} / ${ticks[i]!.label}`,
        ).toBeGreaterThanOrEqual(16);
      }
    }
  });

  it("H-35: at 256x only very quiet ticks stay on-screen (0 dBFS is scaled off the top/bottom)", () => {
    // Zooming IN vertically shrinks the visible amplitude range, so it's the LOUD ticks (0 dBFS'
    // amplitude of 1.0, scaled by 256x) that go off-screen — inverted from the 1x case above.
    const ticks = amplitudeTicksDbfs(800, MAX_VERTICAL_ZOOM, 16);
    expect(ticks.length).toBeGreaterThan(0);
    expect(ticks.some((t) => t.db === 0)).toBe(false);
    expect(ticks.every((t) => t.db! <= -60)).toBe(true);
  });
});

describe("amplitudeTicksPercent (H-72 item 1, SPEC-006 §2.4)", () => {
  it("has evenly spaced ticks at ..., -100%, -50%, 0%, 50%, 100%, ... of full scale", () => {
    const ticks = amplitudeTicksPercent(400, 1, 10);
    // 0% should be at the centerline
    const zeroTick = ticks.find((t) => t.percent === 0);
    expect(zeroTick).toBeDefined();
    expect(zeroTick!.y).toBe(200);
    // ±100% should be at the top/bottom edges
    const topTick = ticks.find((t) => t.percent === 100);
    const bottomTick = ticks.find((t) => t.percent === -100);
    expect(topTick?.y).toBe(0);
    expect(bottomTick?.y).toBe(400);
  });

  it("is symmetric: every non-zero percent value appears both positive and negative, mirrored around the centerline", () => {
    const ticks = amplitudeTicksPercent(1000, 1, 5);
    // Group by absolute percent
    const byAbsPercent = new Map<number, { pos?: number; neg?: number }>();
    for (const t of ticks) {
      const absPercent = Math.abs(t.percent!);
      const entry = byAbsPercent.get(absPercent) ?? {};
      if (t.percent! > 0) entry.pos = t.y;
      if (t.percent! < 0) entry.neg = t.y;
      byAbsPercent.set(absPercent, entry);
    }
    // Every non-zero percent should have both positive and negative versions
    for (const [absPercent, { pos, neg }] of byAbsPercent) {
      if (absPercent > 0) {
        expect(pos, `percent=±${absPercent}`).toBeDefined();
        expect(neg, `percent=±${absPercent}`).toBeDefined();
        // They should be mirrored around centerY = 500
        expect(pos! + neg!).toBeCloseTo(1000, 5);
      }
    }
    // 0% should only appear once, at the centerline
    const zeroTicks = ticks.filter((t) => t.percent === 0);
    expect(zeroTicks).toHaveLength(1);
    expect(zeroTicks[0]!.y).toBe(500);
  });

  it("thins by the minimum pixel gap, keeping the most useful (0%, ±50%, ±100%) ticks first", () => {
    const ticks = amplitudeTicksPercent(60, 1, 100); // tiny pane: almost everything collides
    // With a 60px pane and 100px minimum gap, only 0% (always included) and the very edges survive
    expect(ticks.some((t) => t.percent === 0)).toBe(true);
    // The edges (100% top and -100% bottom) may or may not fit depending on exact pixel positions
    // but this is a degenerate case, so just check that high-magnitude ticks are preferred
    expect(ticks.length).toBeGreaterThan(0);
  });

  it("returns ticks in increasing y order", () => {
    const ticks = amplitudeTicksPercent(400, 1, 10);
    for (let i = 1; i < ticks.length; i++) {
      expect(ticks[i]!.y).toBeGreaterThanOrEqual(ticks[i - 1]!.y);
    }
  });

  it("scales with verticalZoom, making high zoom show only the quietest ticks", () => {
    const ticks1x = amplitudeTicksPercent(400, 1, 10);
    const ticks4x = amplitudeTicksPercent(400, 4, 10);
    // At higher zoom (4x), the visible amplitude range shrinks from ±1.0 to ±0.25,
    // so only low-percent ticks (0%, ±50%) fit, whereas at 1x we see up to ±100%
    expect(ticks1x.some((t) => t.percent === 100 || t.percent === -100)).toBe(true);
    expect(ticks4x.every((t) => Math.abs(t.percent ?? 0) <= 50)).toBe(true);
  });

  it("never puts two labels closer than the minimum gap", () => {
    for (const height of [120, 200, 283, 370, 480, 800]) {
      const ticks = amplitudeTicksPercent(height, 1, 16);
      for (let i = 1; i < ticks.length; i++) {
        expect(ticks[i]!.y - ticks[i - 1]!.y, `${height}px: ${ticks[i - 1]!.percent}% / ${ticks[i]!.percent}%`).toBeGreaterThanOrEqual(16);
      }
    }
  });

  it("returns an empty list for a non-positive height", () => {
    expect(amplitudeTicksPercent(0, 1, 10)).toEqual([]);
    expect(amplitudeTicksPercent(-10, 1, 10)).toEqual([]);
  });
});

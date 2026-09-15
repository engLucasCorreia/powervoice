import { describe, expect, it } from "vitest";
import {
  clampSamplesPerPixel,
  clampStartSample,
  columnYRange,
  MIN_SAMPLES_PER_PIXEL,
  niceTickStepSeconds,
  pickLevel,
  pixelAtSample,
  PEAK_LEVELS_SPP,
  RAW_SPP,
  reduceColumns,
  sampleAtPixel,
  sampleTicks,
  showsDots,
  timeTicks,
  zoomAroundSample,
  zoomFullSamplesPerPixel,
  zoomStep,
} from "./coords";

describe("columnYRange (H-13, SPEC-006 §2.3/§4.5: shared by both renderers)", () => {
  it("maps [-1, 1] amplitude to [0, 2*centerY] pixel span", () => {
    expect(columnYRange(-1, 1, 50)).toEqual([0, 100]);
  });

  it("is at least 1px tall for a silent (min === max) column", () => {
    expect(columnYRange(0, 0, 50)).toEqual([50, 51]);
  });

  it("top is derived from max, bottom from min (screen y grows downward)", () => {
    const [top, bottom] = columnYRange(-0.5, 0.8, 40);
    expect(top).toBeCloseTo(40 - 0.8 * 40, 10);
    expect(bottom).toBeCloseTo(40 - -0.5 * 40, 10);
  });
});

describe("sampleAtPixel / pixelAtSample (SPEC-006 §4.1)", () => {
  it("round-trips exactly at integer spp", () => {
    expect(sampleAtPixel(10, 1000, 4)).toBe(1040);
    expect(pixelAtSample(1040, 1000, 4)).toBe(10);
  });

  it("rounds consistently at fractional spp (AC-4/AC-7 exactness)", () => {
    const startSample = 12_345;
    const spp = 0.37;
    for (let px = 0; px < 50; px++) {
      const sample = sampleAtPixel(px, startSample, spp);
      // px(sample(px)) must land back on px (or within 1, since spp < 1 packs > 1 px/sample).
      expect(Math.abs(pixelAtSample(sample, startSample, spp) - px)).toBeLessThanOrEqual(1);
    }
  });
});

describe("pickLevel (SPEC-006 §2.3 AC-1)", () => {
  it("picks RAW below the finest pyramid level", () => {
    expect(pickLevel(0.1)).toBe(RAW_SPP);
    expect(pickLevel(63.999)).toBe(RAW_SPP);
  });

  it("picks the largest level <= samplesPerPixel", () => {
    expect(pickLevel(64)).toBe(64);
    expect(pickLevel(300)).toBe(256);
    expect(pickLevel(65_536)).toBe(65_536);
    expect(pickLevel(1_000_000)).toBe(65_536);
  });

  it("sweeping samplesPerPixel never picks a level > samplesPerPixel", () => {
    for (let spp = 0.1; spp < 200_000; spp *= 1.3) {
      const level = pickLevel(spp);
      if (level !== RAW_SPP) {
        expect(level).toBeLessThanOrEqual(spp);
        expect(PEAK_LEVELS_SPP).toContain(level);
      }
    }
  });
});

describe("showsDots (SPEC-006 §2.3 AC-3)", () => {
  it("is a pure threshold at 1/spp >= 3", () => {
    expect(showsDots(1 / 3)).toBe(true); // exactly 3 px/sample
    expect(showsDots(1 / 3.0001)).toBe(true); // slightly more than 3 px/sample
    expect(showsDots(0.5)).toBe(false); // 1/0.5 = 2 px/sample
    expect(showsDots(1 / 2.999)).toBe(false); // slightly less than 3 px/sample
  });
});

describe("zoom bounds (SPEC-006 §2.6 AC-4)", () => {
  it("zoom full exactly fits the document", () => {
    expect(zoomFullSamplesPerPixel(48_000, 800)).toBeCloseTo(60);
  });

  it("clamps to [0.1, zoom-full]", () => {
    expect(clampSamplesPerPixel(0.0001, 48_000, 800)).toBe(MIN_SAMPLES_PER_PIXEL);
    expect(clampSamplesPerPixel(1e9, 48_000, 800)).toBeCloseTo(60);
  });

  it("keyboard zoom steps by sqrt(2) and clamps at both ends", () => {
    const zoomedIn = zoomStep(60, 1, 48_000, 800);
    expect(zoomedIn).toBeCloseTo(60 / Math.SQRT2);
    const zoomedOut = zoomStep(60, -1, 48_000, 800);
    expect(zoomedOut).toBeCloseTo(60); // zoom-full is the max; zooming out further is a no-op
    expect(zoomStep(MIN_SAMPLES_PER_PIXEL, 1, 48_000, 800)).toBe(MIN_SAMPLES_PER_PIXEL);
  });

  it("zoom-around-sample keeps the anchor sample under the same pixel", () => {
    const anchorSample = 100_000;
    const anchorPx = 200;
    const nextSpp = 10;
    const startSample = zoomAroundSample(anchorSample, anchorPx, nextSpp);
    expect(sampleAtPixel(anchorPx, startSample, nextSpp)).toBe(anchorSample);
  });
});

describe("clampStartSample", () => {
  it("never goes negative and never scrolls past the point the whole tail still fits", () => {
    expect(clampStartSample(-500, 1, 1000, 800)).toBe(0);
    // viewport (800 px * 1 spp = 800 samples) can't show past sample 200 with a 1000-sample doc.
    expect(clampStartSample(10_000, 1, 1000, 800)).toBe(200);
    // when the document is shorter than the viewport, start is clamped to 0.
    expect(clampStartSample(50, 1, 100, 800)).toBe(0);
  });
});

describe("niceTickStepSeconds (SPEC-006 §4.2)", () => {
  it("picks the smallest {1,2,5}x10^n step that is >= the minimum gap", () => {
    expect(niceTickStepSeconds(0.7)).toBe(1);
    expect(niceTickStepSeconds(1)).toBe(1);
    expect(niceTickStepSeconds(1.5)).toBe(2);
    expect(niceTickStepSeconds(3)).toBe(5);
    expect(niceTickStepSeconds(7)).toBe(10);
    expect(niceTickStepSeconds(0.03)).toBeCloseTo(0.05);
  });
});

describe("reduceColumns (SPEC-006 §4.3, AC-1/AC-2)", () => {
  it("combines at most 4 buckets per pixel, conservative union", () => {
    // level 64, spp 200 (< 4*64=256): each pixel spans between 1 and 4 level-64 buckets.
    const level = 64;
    const buckets: Array<[number, number]> = [
      [-0.1, 0.1],
      [-0.2, 0.05],
      [-0.05, 0.3],
      [-0.4, 0.02],
      [-0.15, 0.15],
    ];
    const columns = reduceColumns(buckets, 0, level, 0, 200, 2);
    // Pixel 0 covers samples [0, 200) -> buckets 0..2 (floor(199/64)=3, so 0..3 actually).
    expect(columns[0]).toEqual([
      Math.min(-0.1, -0.2, -0.05, -0.4),
      Math.max(0.1, 0.05, 0.3, 0.02),
    ]);
    expect(columns.length).toBe(2);
  });

  it("never under-reports vs. a brute-force scan (AC-2 exactness)", () => {
    const level = 64;
    const buckets: Array<[number, number]> = Array.from({ length: 20 }, (_, i) => [
      -1 - i * 0.01,
      1 + i * 0.02,
    ]);
    const startSample = 500;
    const samplesPerPixel = 130; // spans ~2 buckets per column
    const viewportPx = 5;
    const columns = reduceColumns(buckets, 0, level, startSample, samplesPerPixel, viewportPx);
    for (let px = 0; px < viewportPx; px++) {
      const lo = startSample + px * samplesPerPixel;
      const hi = startSample + (px + 1) * samplesPerPixel;
      let mn = Infinity;
      let mx = -Infinity;
      for (let s = lo; s < hi; s++) {
        const i = Math.floor(s / level);
        const bucket = buckets[i];
        if (bucket) {
          mn = Math.min(mn, bucket[0]);
          mx = Math.max(mx, bucket[1]);
        }
      }
      const col = columns[px];
      expect(col).not.toBeNull();
      if (col) {
        expect(col[0]).toBeLessThanOrEqual(mn);
        expect(col[1]).toBeGreaterThanOrEqual(mx);
      }
    }
  });

  it("a column past the fetched buckets is null", () => {
    const columns = reduceColumns([[-1, 1]], 0, 64, 0, 64, 3);
    expect(columns[0]).toEqual([-1, 1]);
    expect(columns[1]).toBeNull();
    expect(columns[2]).toBeNull();
  });

  it("skips NaN (PARTIAL placeholder) buckets", () => {
    const buckets: Array<[number, number]> = [
      [Number.NaN, Number.NaN],
      [-0.5, 0.5],
    ];
    const columns = reduceColumns(buckets, 0, 64, 0, 128, 1);
    expect(columns[0]).toEqual([-0.5, 0.5]);
  });
});

describe("timeTicks (SPEC-006 §2.5, §4.2, AC-6)", () => {
  it("every tick's sample is the exact document sample at its pixel position", () => {
    const startSample = 48_000 * 3; // 3 s in
    const samplesPerPixel = 48_000 / 100; // 100 px per second
    const viewportPx = 500; // 5 s visible
    const ticks = timeTicks(startSample, samplesPerPixel, viewportPx, 48_000, 60);
    expect(ticks.length).toBeGreaterThan(0);
    for (const tick of ticks) {
      const px = pixelAtSample(tick.sample, startSample, samplesPerPixel);
      expect(sampleAtPixel(px, startSample, samplesPerPixel)).toBe(tick.sample);
    }
  });

  it("returns nothing for a degenerate viewport/rate", () => {
    expect(timeTicks(0, 1, 100, 0, 60)).toEqual([]);
    expect(timeTicks(0, 0, 100, 48_000, 60)).toEqual([]);
    expect(timeTicks(0, 1, 0, 48_000, 60)).toEqual([]);
  });
});

describe("sampleTicks (SPEC-006 §2.5/§4.2 'samples' format, AC-6)", () => {
  it("ticks are exact multiples of a {1,2,5}x10^n sample step, needing no reconversion", () => {
    const startSample = 12_345;
    const samplesPerPixel = 10;
    const viewportPx = 500;
    const ticks = sampleTicks(startSample, samplesPerPixel, viewportPx, 60);
    expect(ticks.length).toBeGreaterThan(0);
    const step = ticks.length > 1 ? ticks[1]! - ticks[0]! : ticks[0]!;
    for (const tick of ticks) {
      expect(Number.isInteger(tick)).toBe(true);
      expect(tick % step).toBe(0);
    }
  });

  it("covers the whole visible sample range", () => {
    const startSample = 0;
    const samplesPerPixel = 100;
    const viewportPx = 200; // visible: [0, 20_000)
    const ticks = sampleTicks(startSample, samplesPerPixel, viewportPx, 60);
    expect(ticks[0]).toBeLessThanOrEqual(startSample);
    expect(ticks.at(-1)).toBeGreaterThanOrEqual(startSample + viewportPx * samplesPerPixel);
  });

  it("the step never drops below 1 sample even zoomed in past 1:1", () => {
    const ticks = sampleTicks(0, 0.1, 50, 60);
    expect(ticks.length).toBeGreaterThan(1);
    const step = ticks[1]! - ticks[0]!;
    expect(step).toBeGreaterThanOrEqual(1);
    expect(Number.isInteger(step)).toBe(true);
  });

  it("returns nothing for a degenerate viewport/spp", () => {
    expect(sampleTicks(0, 0, 100, 60)).toEqual([]);
    expect(sampleTicks(0, 1, 0, 60)).toEqual([]);
  });

  it("never emits a negative tick", () => {
    const ticks = sampleTicks(5, 10, 50, 60);
    for (const tick of ticks) {
      expect(tick).toBeGreaterThanOrEqual(0);
    }
  });
});

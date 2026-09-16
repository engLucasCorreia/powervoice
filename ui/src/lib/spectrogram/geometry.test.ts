import { describe, expect, it } from "vitest";
import {
  CANVAS2D_COLUMN_BIN_BUDGET,
  FFT_SIZES,
  MAX_TILES_PER_REQUEST,
  TILE_FRAMES,
  autoFftSize,
  canvasColumnStride,
  frameCenterSample,
  frameColumnBounds,
  frameLinearMapping,
  hopForZoom,
  isOverview,
  tileCount,
  tileDevicePxRange,
  tilesForView,
  totalFrames,
  visibleTileIndices,
} from "./geometry";

describe("spectrogram geometry (SPEC-007 §2.6, §4.3)", () => {
  it("resolves Auto FFT sizes per sample rate (AC-14)", () => {
    expect(autoFftSize(44_100)).toBe(2048);
    expect(autoFftSize(48_000)).toBe(2048);
    expect(autoFftSize(88_200)).toBe(4096);
    expect(autoFftSize(96_000)).toBe(4096);
    expect(autoFftSize(192_000)).toBe(8192);
    expect(autoFftSize(22_050)).toBe(1024);
    expect(autoFftSize(8_000)).toBe(256);
  });

  it("follows hop = max(pow2_floor(sppDev), N/16) over a zoom sweep (AC-6)", () => {
    for (const n of FFT_SIZES) {
      for (let spp = 0.1; spp <= 2e5; spp *= 1.07) {
        const hop = hopForZoom(spp, n);
        expect(Number.isInteger(Math.log2(hop))).toBe(true);
        expect(hop).toBeGreaterThanOrEqual(n / 16);
        if (spp >= 1 && hop > n / 16) {
          expect(hop).toBeLessThanOrEqual(spp);
          expect(spp).toBeLessThan(2 * hop);
        }
      }
    }
    expect(hopForZoom(1000, 2048)).toBe(512);
    expect(hopForZoom(0.1, 2048)).toBe(128);
    expect(hopForZoom(Number.NaN, 2048)).toBe(128);
    expect(hopForZoom(90_000, 2048)).toBe(65_536);
    expect(isOverview(2048, 4096)).toBe(true);
    expect(isOverview(2048, 2048)).toBe(false);
  });

  it("anchors the frame grid at sample 0 with 256-frame tiles", () => {
    expect(totalFrames(480_000, 128)).toBe(3750);
    expect(tileCount(480_000, 128)).toBe(15);
    expect(tileCount(0, 128)).toBe(0);
    expect(frameCenterSample(2, 0, 512)).toBe(2 * 256 * 512);
    expect(frameCenterSample(2, 5, 512)).toBe((2 * 256 + 5) * 512);
  });

  it("lists visible tiles first, then neighbours nearest first, clipped and capped", () => {
    const hop = 512;
    const span = 256 * hop; // 131 072 samples per tile
    const len = 40 * span;
    // View exactly covering tiles 10..11, one viewport (2 tiles) each side.
    const tiles = tilesForView(10 * span + hop, 12 * span - hop, len, hop);
    expect(tiles.slice(0, 2)).toEqual([10, 11]);
    expect(tiles.slice(2)).toEqual([12, 9, 13, 8]);
    // At the document start there's nothing to the left.
    expect(tilesForView(0, span - hop, len, hop)).toEqual([0, 1]);
    // At the end nothing to the right.
    expect(tilesForView(39 * span + hop, 40 * span, len, hop)).toEqual([39, 38]);
    // Whole-file views are capped.
    expect(tilesForView(0, 100 * 40 * span, 100 * 40 * span, hop)).toHaveLength(
      MAX_TILES_PER_REQUEST,
    );
    expect(tilesForView(0, 100, 0, hop)).toEqual([]);
  });

  it("frameColumnBounds draws one column per device pixel at devicePixelRatio 2 (H-12 HiDPI)", () => {
    const startSample = 0;
    const samplesPerPixel = 10; // CSS-pixel (document) units
    const hop = 64;

    // A 100 CSS-pixel-wide viewport is 100 device-pixel columns at dpr 1, and 200 at dpr 2 — the
    // column count follows the *backing* (device-pixel) width, not the CSS width.
    const dpr1 = frameColumnBounds(100, startSample, samplesPerPixel, 1, hop);
    const dpr2 = frameColumnBounds(200, startSample, samplesPerPixel, 2, hop);
    expect(dpr1.lo.length).toBe(100);
    expect(dpr2.lo.length).toBe(200);

    // Each dpr-2 column spans half the samples (and so half the frames) of a dpr-1 column, since
    // there are twice as many columns covering the same CSS-pixel span.
    const spanDpr1 = dpr1.hi[0]! - dpr1.lo[0]!;
    const spanDpr2 = dpr2.hi[0]! - dpr2.lo[0]!;
    expect(spanDpr2).toBeCloseTo(spanDpr1 / 2, 10);

    // The two device-pixel columns covering dpr-1's first CSS pixel span the same total range as
    // that one CSS-pixel column (device pixels subdivide, they don't change the covered range).
    expect(dpr2.lo[0]).toBeCloseTo(dpr1.lo[0]!, 10);
    expect(dpr2.hi[1]).toBeCloseTo(dpr1.hi[0]!, 10);

    // A later column starts where the document position has advanced accordingly.
    expect(dpr2.lo[10]).toBeCloseTo((startSample + 5 * samplesPerPixel) / hop, 10);
  });

  it("canvasColumnStride is 1 below the budget and grows just enough to stay under it (H-54)", () => {
    // The passing 1280×720 case (680×219-ish device pane, FFT Auto 2048 → 1025 bins) stays full
    // resolution: well under budget.
    expect(canvasColumnStride(680, 1025)).toBe(1);
    // Right at the budget: still stride 1 (the guard is "above", not "at or above").
    expect(canvasColumnStride(CANVAS2D_COLUMN_BIN_BUDGET / 1025, 1025)).toBe(1);
    // One unit over budget rounds up to stride 2.
    expect(canvasColumnStride(CANVAS2D_COLUMN_BIN_BUDGET / 1025 + 1, 1025)).toBe(2);
    // The failing 2126×850 case (1526×219 device pane, 1025 bins) is reduced, not left at 1.
    const failingStride = canvasColumnStride(1526, 1025);
    expect(failingStride).toBeGreaterThan(1);
    // Grouping backingWidthPx device columns into stride-sized groups always covers every column
    // (the last group may be a partial, smaller group) and never groups more than `stride`.
    for (const [backingWidthPx, bins] of [[1526, 1025], [2126, 2049], [90, 129]] as const) {
      const stride = canvasColumnStride(backingWidthPx, bins);
      const groups = Math.ceil(backingWidthPx / stride);
      expect(groups * stride).toBeGreaterThanOrEqual(backingWidthPx);
      expect((groups - 1) * stride).toBeLessThan(backingWidthPx);
    }
    // Degenerate inputs never return a non-positive or non-finite stride.
    expect(canvasColumnStride(0, 1025)).toBe(1);
    expect(canvasColumnStride(1526, 0)).toBe(1);
    expect(canvasColumnStride(-10, 1025)).toBe(1);
  });
});

describe("frameLinearMapping / visibleTileIndices / tileDevicePxRange (H-13, WebGL2 renderer)", () => {
  it("frameLinearMapping's frame(px) formula matches frameColumnBounds's per-column values", () => {
    const startSample = 12_345;
    const samplesPerPixel = 37;
    const dpr = 2;
    const hop = 128;
    const backingWidthPx = 50;
    const { frameAtPx0, framesPerPx } = frameLinearMapping(startSample, samplesPerPixel, dpr, hop);
    const { lo, hi } = frameColumnBounds(backingWidthPx, startSample, samplesPerPixel, dpr, hop);
    for (let px = 0; px < backingWidthPx; px++) {
      expect(frameAtPx0 + framesPerPx * px).toBeCloseTo(lo[px]!, 9);
      expect(frameAtPx0 + framesPerPx * (px + 1)).toBeCloseTo(hi[px]!, 9);
    }
  });

  it("tileDevicePxRange partitions [0, backingWidthPx) with no gap or overlap between tiles", () => {
    const frameAtPx0 = 0;
    const framesPerPx = 3.3;
    const backingWidthPx = 400;
    const count = 5;
    let prevX1 = 0;
    for (let k = 0; k < count; k++) {
      const { x0, x1 } = tileDevicePxRange(k, frameAtPx0, framesPerPx, backingWidthPx);
      expect(x0).toBe(prevX1);
      expect(x1).toBeGreaterThanOrEqual(x0);
      prevX1 = x1;
    }
  });

  it("tileDevicePxRange is the exact inverse of frame(px) at each tile boundary", () => {
    const frameAtPx0 = -50; // a negative frameAtPx0 is normal (startSample can be 0, hop large)
    const framesPerPx = 0.7;
    const backingWidthPx = 1000;
    const { x0 } = tileDevicePxRange(2, frameAtPx0, framesPerPx, backingWidthPx);
    const frameAtX0 = frameAtPx0 + framesPerPx * x0;
    expect(Math.abs(frameAtX0 - 2 * TILE_FRAMES)).toBeLessThanOrEqual(framesPerPx / 2 + 1e-9);
  });

  it("tileDevicePxRange clamps to [0, backingWidthPx]", () => {
    const { x0, x1 } = tileDevicePxRange(0, 10_000, 1, 100);
    expect(x0).toBe(0);
    expect(x1).toBe(0);
  });

  it("visibleTileIndices covers every tile whose quad could touch [0, backingWidthPx)", () => {
    const frameAtPx0 = 0;
    const framesPerPx = 2;
    const backingWidthPx = 300;
    const count = 20;
    const visible = visibleTileIndices(frameAtPx0, framesPerPx, backingWidthPx, count);
    // Every tile whose device-pixel range actually intersects the backing width must be included.
    for (let k = 0; k < count; k++) {
      const { x0, x1 } = tileDevicePxRange(k, frameAtPx0, framesPerPx, backingWidthPx);
      if (x1 > x0) {
        expect(visible).toContain(k);
      }
    }
  });

  it("visibleTileIndices is empty for an empty document or a zero-width canvas", () => {
    expect(visibleTileIndices(0, 1, 100, 0)).toEqual([]);
    expect(visibleTileIndices(0, 1, 0, 10)).toEqual([]);
  });
});

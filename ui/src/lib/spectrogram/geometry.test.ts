import { describe, expect, it } from "vitest";
import {
  FFT_SIZES,
  MAX_TILES_PER_REQUEST,
  autoFftSize,
  frameCenterSample,
  hopForZoom,
  isOverview,
  tileCount,
  tilesForView,
  totalFrames,
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
});

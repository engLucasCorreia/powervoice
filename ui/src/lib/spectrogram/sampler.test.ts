import { describe, expect, it } from "vitest";
import { TILE_FRAMES } from "./geometry";
import { codeAtGrid, dbAtGrid, nearestCode, pixelDb, sampleGridSpan, type TileLookup } from "./sampler";
import type { SpectroTile } from "./spectroRequester";
import { dequantizeDb } from "./vxst";

/** A tile of `frames × bins` codes, all `fill`, with `overrides` poking specific `(frame, bin)`
 * cells (frame-major, matching `VXST`'s payload layout). */
function makeTile(opts: {
  tileIndex?: number;
  frames: number;
  bins: number;
  fill?: number;
  overrides?: Array<{ frame: number; bin: number; code: number }>;
}): SpectroTile {
  const { tileIndex = 0, frames, bins, fill = 0, overrides = [] } = opts;
  const data = new Uint8Array(frames * bins).fill(fill);
  for (const { frame, bin, code } of overrides) {
    data[frame * bins + bin] = code;
  }
  return {
    fftSize: (bins - 1) * 2,
    hop: 512,
    tileIndex,
    firstFrameCenterSample: tileIndex * TILE_FRAMES * 512,
    frames,
    bins,
    preview: false,
    audioRev: 0,
    data,
  };
}

describe("codeAtGrid / dbAtGrid (SPEC-007 §4.2, §4.3)", () => {
  it("reads the exact code at (frame, bin) from the owning tile", () => {
    const tile = makeTile({ frames: 4, bins: 5, fill: 10, overrides: [{ frame: 2, bin: 3, code: 200 }] });
    const lookup: TileLookup = (i) => (i === 0 ? tile : undefined);
    expect(codeAtGrid(lookup, 2, 3)).toBe(200);
    expect(codeAtGrid(lookup, 0, 0)).toBe(10);
    expect(dbAtGrid(lookup, 2, 3)).toBeCloseTo(dequantizeDb(200), 6);
  });

  it("routes to the correct tile by frame >= TILE_FRAMES", () => {
    const tile0 = makeTile({ tileIndex: 0, frames: 256, bins: 3, fill: 1 });
    const tile1 = makeTile({ tileIndex: 1, frames: 256, bins: 3, fill: 2 });
    const lookup: TileLookup = (i) => [tile0, tile1][i];
    expect(codeAtGrid(lookup, 0, 0)).toBe(1);
    expect(codeAtGrid(lookup, 255, 0)).toBe(1);
    expect(codeAtGrid(lookup, 256, 0)).toBe(2);
    expect(codeAtGrid(lookup, 511, 0)).toBe(2);
  });

  it("returns null for a tile that hasn't arrived, or out-of-bounds frame/bin", () => {
    const tile = makeTile({ frames: 4, bins: 5 });
    const lookup: TileLookup = (i) => (i === 0 ? tile : undefined);
    expect(codeAtGrid(lookup, 0, 0)).toBe(0);
    expect(codeAtGrid(() => undefined, 0, 0)).toBeNull();
    expect(codeAtGrid(lookup, 10, 0)).toBeNull(); // frame beyond this (short, final) tile
    expect(codeAtGrid(lookup, 0, 10)).toBeNull(); // bin beyond bins
    expect(codeAtGrid(lookup, -1, 0)).toBeNull();
  });
});

describe("sampleGridSpan (SPEC-007 §2.3/§2.4 max-or-interpolate rule)", () => {
  const values = [10, 20, 5, 40, 15];
  const at = (i: number): number | null => values[i] ?? null;

  it("takes the maximum when the span covers >= 1 whole step", () => {
    expect(sampleGridSpan(5, 0, 3, at)).toBe(20); // covers indices 0,1,2 -> max(10,20,5)
    expect(sampleGridSpan(5, 1, 5, at)).toBe(40); // covers 1..4 -> max(20,5,40,15)
  });

  it("interpolates linearly between the two bracketing points when the span is < 1 step", () => {
    // Span centred at 1.5 -> halfway between index 1 (20) and 2 (5).
    expect(sampleGridSpan(5, 1.3, 1.7, at)).toBeCloseTo(12.5, 6);
    // Span centred exactly at an integer index returns that point.
    expect(sampleGridSpan(5, 1.95, 2.05, at)).toBeCloseTo(at(2)!, 1);
  });

  it("skips unknown points in the max, and falls back to the known neighbour when interpolating", () => {
    const sparse = (i: number): number | null => (i === 1 ? null : values[i] ?? null);
    expect(sampleGridSpan(5, 0, 3, sparse)).toBe(10); // max(10, null, 5) -> 10
    expect(sampleGridSpan(5, 1.3, 1.7, sparse)).toBe(5); // interpolate(null, 5) -> 5
    expect(sampleGridSpan(5, 0, 5, () => null)).toBeNull();
  });

  it("is null for a degenerate or out-of-range span", () => {
    expect(sampleGridSpan(0, 0, 1, at)).toBeNull();
    expect(sampleGridSpan(5, 3, 3, at)).toBeNull();
    expect(sampleGridSpan(5, -5, -1, at)).toBeNull();
  });
});

describe("pixelDb (SPEC-007 §4.7, AC-11 row maximum)", () => {
  it("a single hot bin is shown exactly by whichever pixel's span covers it (row max)", () => {
    // 1 frame, 10 bins; bin 7 is the lone hot bin (code 255 -> +6 dB), everything else silent.
    const tile = makeTile({ frames: 1, bins: 10, fill: 0, overrides: [{ frame: 0, bin: 7, code: 255 }] });
    const lookup: TileLookup = (i) => (i === 0 ? tile : undefined);
    // A pixel row spanning bins [5, 10) covers the hot bin -> max rule picks it up exactly.
    const db = pixelDb(lookup, 1, 10, 0, 1, 5, 10);
    expect(db).toBeCloseTo(dequantizeDb(255), 6);
    // A row spanning only [0, 5) misses it -> reads the silent floor code (0).
    const missed = pixelDb(lookup, 1, 10, 0, 1, 0, 5);
    expect(missed).toBeCloseTo(dequantizeDb(0), 6);
  });

  it("interpolates in both axes when zoomed in past one frame/bin per pixel", () => {
    const tile = makeTile({
      frames: 2,
      bins: 2,
      overrides: [
        { frame: 0, bin: 0, code: 0 },
        { frame: 0, bin: 1, code: 100 },
        { frame: 1, bin: 0, code: 200 },
        { frame: 1, bin: 1, code: 255 },
      ],
    });
    const lookup: TileLookup = (i) => (i === 0 ? tile : undefined);
    // Centre of the 2x2 grid: bilinear-ish average of all four corners via nested interpolation.
    const db = pixelDb(lookup, 2, 2, 0.45, 0.55, 0.45, 0.55);
    const expected =
      (dequantizeDb(0) + dequantizeDb(100) + dequantizeDb(200) + dequantizeDb(255)) / 4;
    expect(db).toBeCloseTo(expected, 1);
  });

  it("returns null where no tile has arrived", () => {
    expect(pixelDb(() => undefined, 10, 10, 0, 1, 0, 1)).toBeNull();
  });
});

describe("nearestCode (SPEC-007 §2.7 hover: nearest bin in the nearest frame)", () => {
  it("picks the single nearest cell, never the max/interpolate render value", () => {
    const tile = makeTile({
      frames: 2,
      bins: 2,
      overrides: [
        { frame: 0, bin: 0, code: 0 },
        { frame: 0, bin: 1, code: 100 },
        { frame: 1, bin: 0, code: 200 },
        { frame: 1, bin: 1, code: 255 },
      ],
    });
    const lookup: TileLookup = (i) => (i === 0 ? tile : undefined);
    // A point near (0.4, 0.4) rounds to (frame 0, bin 0) -> code 0, not some blended value.
    expect(nearestCode(lookup, 2, 2, 0.4, 0.4)).toBe(0);
    // A point near (0.6, 0.6) rounds to (frame 1, bin 1) -> code 255.
    expect(nearestCode(lookup, 2, 2, 0.6, 0.6)).toBe(255);
  });

  it("clamps out-of-range frame/bin to the valid grid", () => {
    const tile = makeTile({ frames: 3, bins: 3, fill: 42 });
    const lookup: TileLookup = (i) => (i === 0 ? tile : undefined);
    expect(nearestCode(lookup, 3, 3, -5, -5)).toBe(42);
    expect(nearestCode(lookup, 3, 3, 500, 500)).toBe(42);
  });

  it("returns null when nothing is known and null for an empty grid", () => {
    expect(nearestCode(() => undefined, 3, 3, 0, 0)).toBeNull();
    expect(nearestCode(() => undefined, 0, 0, 0, 0)).toBeNull();
  });
});

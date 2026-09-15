import { describe, expect, it } from "vitest";
import { TILE_FRAMES } from "./geometry";
import { columnDb, createColumnScratch, pixelDb, type TileLookup } from "./sampler";
import type { SpectroTile } from "./spectroRequester";

/** A deterministic PRNG (mulberry32), so a failure reproduces. */
function rng(seed: number): () => number {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

function tileSet(random: () => number, count: number, bins: number, missing: Set<number>): TileLookup {
  const tiles = new Map<number, SpectroTile>();
  for (let k = 0; k < count; k++) {
    if (missing.has(k)) continue;
    const data = new Uint8Array(TILE_FRAMES * bins);
    for (let i = 0; i < data.length; i++) data[i] = Math.floor(random() * 256);
    tiles.set(k, {
      fftSize: (bins - 1) * 2,
      hop: 512,
      tileIndex: k,
      firstFrameCenterSample: k * TILE_FRAMES * 512,
      frames: k === count - 1 ? 100 : TILE_FRAMES,
      bins,
      preview: false,
      audioRev: 1,
      data,
    });
  }
  return (k) => tiles.get(k);
}

/**
 * T-704: the Canvas2D spectral renderer now computes each column with `columnDb` (the time rule
 * memoized per bin) instead of `pixelDb` per pixel. It must produce exactly `pixelDb`'s values —
 * SPEC-007 AC-13 makes the hover readout and the renderer share this lookup.
 */
describe("columnDb (T-704)", () => {
  it("equals pixelDb for every row, across sub-frame and multi-frame spans, sub-bin and multi-bin rows, and missing tiles", () => {
    const random = rng(0x7704);
    const bins = 129;
    const tileCount = 6;
    const totalFrames = (tileCount - 1) * TILE_FRAMES + 100;
    const lookup = tileSet(random, tileCount, bins, new Set([2]));
    const rows = 90;
    const scratch = createColumnScratch(bins);
    const out = new Float64Array(rows);
    for (let column = 0; column < 400; column++) {
      const width = [0.3, 0.9, 1, 2.5, 7][column % 5]!;
      const frameLo = random() * (totalFrames + 20) - 10;
      const frameHi = frameLo + width;
      const binLo = new Float64Array(rows);
      const binHi = new Float64Array(rows);
      for (let py = 0; py < rows; py++) {
        const lo = random() * (bins + 4) - 2;
        binLo[py] = lo;
        binHi[py] = lo + [0.2, 0.7, 1, 3.3, 12][py % 5]!;
      }
      columnDb(lookup, totalFrames, bins, frameLo, frameHi, binLo, binHi, out, scratch);
      for (let py = 0; py < rows; py++) {
        const want = pixelDb(lookup, totalFrames, bins, frameLo, frameHi, binLo[py]!, binHi[py]!);
        if (want === null) {
          expect(Number.isNaN(out[py]!), `column ${column} row ${py}`).toBe(true);
        } else {
          expect(out[py], `column ${column} row ${py}`).toBe(want);
        }
      }
    }
  });

  it("looks each tile up at most once per column", () => {
    const random = rng(7);
    const bins = 65;
    const base = tileSet(random, 3, bins, new Set());
    let lookups = 0;
    const counted: TileLookup = (k) => {
      lookups += 1;
      return base(k);
    };
    const rows = 50;
    const binLo = Float64Array.from({ length: rows }, (_, i) => (i * bins) / rows);
    const binHi = Float64Array.from({ length: rows }, (_, i) => ((i + 1) * bins) / rows);
    columnDb(counted, 3 * TILE_FRAMES, bins, TILE_FRAMES - 1.5, TILE_FRAMES + 1.5, binLo, binHi, new Float64Array(rows), createColumnScratch(bins));
    expect(lookups).toBeLessThanOrEqual(2);
  });
});

/**
 * Per-pixel dB sampling from held `VXST` tiles (SPEC-007 §2.3, §2.4, §4.7): maps a continuous
 * pixel span, in frames (time axis) or bins (frequency axis), onto the tile grid — **maximum**
 * when the span covers a whole grid step or more, **linear interpolation** in dB otherwise. Pure
 * and canvas-free, so the Canvas2D renderer and the hover readout can share the exact same
 * lookup (SPEC-007 AC-13: both must agree to within quantization).
 */

import { TILE_FRAMES } from "./geometry";
import type { SpectroTile } from "./spectroRequester";
import { dequantizeDb } from "./vxst";

/** Looks up the tile holding frame `tileIndex * TILE_FRAMES .. +TILE_FRAMES`, or `undefined` if
 * it hasn't arrived (SPEC-007 §2.8: shown as pending). */
export type TileLookup = (tileIndex: number) => SpectroTile | undefined;

/** The raw quantization code at exact integer `(frame, bin)` of the whole-document frame grid, or
 * `null` if no tile covers it yet. */
export function codeAtGrid(lookup: TileLookup, frame: number, bin: number): number | null {
  if (frame < 0 || bin < 0) {
    return null;
  }
  const tileIndex = Math.floor(frame / TILE_FRAMES);
  const tile = lookup(tileIndex);
  if (!tile) {
    return null;
  }
  const localFrame = frame - tileIndex * TILE_FRAMES;
  if (localFrame >= tile.frames || bin >= tile.bins) {
    return null;
  }
  return tile.data[localFrame * tile.bins + bin] ?? null;
}

/** Dequantized dB at exact integer `(frame, bin)`, or `null` if unknown. */
export function dbAtGrid(lookup: TileLookup, frame: number, bin: number): number | null {
  const code = codeAtGrid(lookup, frame, bin);
  return code === null ? null : dequantizeDb(code);
}

/**
 * The value covering continuous span `[lo, hi)` over an axis of `count` integer grid points,
 * sampled via `at(i)` for integer `i` (SPEC-007 §2.3 time axis / §2.4 frequency axis): when the
 * span covers **≥ 1** whole grid step, the **maximum** of the covered points ("shows their
 * maximum" / "shows the maximum of those bins"); otherwise **linear interpolation** between the
 * two points bracketing the span's centre. Missing points (`null`) are skipped in the max and
 * fall back to whichever neighbour is known when interpolating; returns `null` only when nothing
 * in range is known.
 */
export function sampleGridSpan(
  count: number,
  lo: number,
  hi: number,
  at: (i: number) => number | null,
): number | null {
  if (count <= 0 || !(hi > lo)) {
    return null;
  }
  const clampedLo = Math.max(0, lo);
  const clampedHi = Math.min(count, hi);
  if (clampedHi <= clampedLo) {
    return null;
  }
  if (clampedHi - clampedLo >= 1) {
    const i0 = Math.max(0, Math.floor(clampedLo));
    const i1 = Math.min(count - 1, Math.ceil(clampedHi) - 1);
    let max: number | null = null;
    for (let i = i0; i <= i1; i++) {
      const v = at(i);
      if (v !== null && (max === null || v > max)) {
        max = v;
      }
    }
    return max;
  }
  const center = Math.min(count - 1, Math.max(0, (lo + hi) / 2));
  const i0 = Math.floor(center);
  const i1 = Math.min(count - 1, i0 + 1);
  const v0 = at(i0);
  const v1 = at(i1);
  if (v0 === null) {
    return v1;
  }
  if (v1 === null) {
    return v0;
  }
  const frac = center - i0;
  return v0 + (v1 - v0) * frac;
}

/**
 * The rendered dB value for one pixel covering frame span `[frameLo, frameHi)` and bin span
 * `[binLo, binHi)` (SPEC-007 §4.7 step 2): the time-axis rule is applied per bin, then the
 * frequency-axis rule combines those across the pixel's bin span — the two rules are independent
 * per their own spec sections, so nesting them is exact.
 */
export function pixelDb(
  lookup: TileLookup,
  totalFrames: number,
  bins: number,
  frameLo: number,
  frameHi: number,
  binLo: number,
  binHi: number,
): number | null {
  const binValue = (bin: number): number | null =>
    sampleGridSpan(totalFrames, frameLo, frameHi, (frame) => dbAtGrid(lookup, frame, bin));
  return sampleGridSpan(bins, binLo, binHi, binValue);
}

/**
 * The hover readout's raw code (SPEC-007 §2.7): the **nearest** bin in the **nearest** frame —
 * deliberately not the max/interpolate render rule above, per §2.7's "nearest bin ... nearest
 * frame". `null` when no tile covers that point yet.
 */
export function nearestCode(
  lookup: TileLookup,
  totalFrames: number,
  bins: number,
  frame: number,
  bin: number,
): number | null {
  if (totalFrames <= 0 || bins <= 0) {
    return null;
  }
  const fi = Math.round(Math.min(Math.max(frame, 0), totalFrames - 1));
  const bi = Math.round(Math.min(Math.max(bin, 0), bins - 1));
  return codeAtGrid(lookup, fi, bi);
}

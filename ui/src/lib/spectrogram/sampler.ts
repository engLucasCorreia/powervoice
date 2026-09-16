/**
 * Per-pixel dB sampling from held `VXST` tiles (SPEC-007 §2.3, §2.4, §4.7): maps a continuous
 * pixel span, in frames (time axis) or bins (frequency axis), onto the tile grid — **maximum**
 * when the span covers a whole grid step or more, **linear interpolation** in dB otherwise. Pure
 * and canvas-free, so the Canvas2D renderer and the hover readout can share the exact same
 * lookup (SPEC-007 AC-13: both must agree to within quantization).
 */

import { TILE_FRAMES } from "./geometry";
import type { SpectroTile } from "./spectroRequester";
import { dequantizeDb, Q_CEIL_DB, Q_FLOOR_DB } from "./vxst";

/** H-47: `dequantizeDb`'s span, so {@link columnDb} can inline it as the identical expression. */
const Q_SPAN_DB = Q_CEIL_DB - Q_FLOOR_DB;
/**
 * H-47: `dequantizeDb(code)` for every one of the 256 codes, precomputed once. A tile code is a
 * `u8`, so this is the whole domain, and each entry is the **same expression** evaluated the same
 * way — the table is bit-exact, not an approximation. It replaces a multiply **and a division**
 * per bin per column (~1.5 M of each per Canvas2D frame of a 2126×850 split view) with one L1
 * load; `x / 255` can't be turned into `x * (1 / 255)` without changing the result, so the
 * division was unavoidable inline.
 */
const CODE_DB = ((): Float64Array => {
  const table = new Float64Array(256);
  for (let code = 0; code < 256; code++) {
    table[code] = Q_FLOOR_DB + (code * Q_SPAN_DB) / 255;
  }
  return table;
})();
/** H-47: {@link columnDb}'s internal "no tile covers this" marker — numeric, so its two hot loops
 * compare instead of calling `Number.isNaN`. Below every possible dB value, and never written to
 * `out` (which keeps `NaN`). */
const UNKNOWN = -Infinity;

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

/** Caller-owned scratch for {@link columnDb}, reused across the columns of a draw. */
export interface ColumnScratch {
  /** Per-bin time-axis dB of the current column (`NaN` = unknown). */
  values: Float64Array;
  /** The column's frame rows: tile data, row offset and bin count (`null` data = no tile). */
  rowData: Array<Uint8Array | null>;
  rowBase: number[];
  rowBins: number[];
}

export function createColumnScratch(bins: number): ColumnScratch {
  return { values: new Float64Array(bins), rowData: [], rowBase: [], rowBins: [] };
}

/**
 * T-704: one pixel column of the Canvas2D renderer. Writes `out[py]` =
 * {@link pixelDb}`(lookup, totalFrames, bins, frameLo, frameHi, binLo[py], binHi[py])` for every
 * row (`NaN` where that is `null`) — the same two §4.7 rules with the same arithmetic — but:
 * - the time rule's frame span is the same for every bin of the column, so its tile rows are
 *   resolved once per column (each tile looked up once) and each bin's value is computed once, not
 *   once per pixel;
 * - "maximum over the covered frames" is taken on the raw codes and dequantized once
 *   (`dequantizeDb` is strictly increasing, so this is exactly the max of the dequantized values);
 * - no closures or allocations per bin; only the bins the rows can reach are computed.
 * The per-pixel version cost ~0.5–3 s per frame on a 60-min document's split view.
 *
 * H-47 (this was 66 % of the self time of every Canvas2D frame of the split view):
 * - "unknown" is `-Infinity` rather than `NaN` inside both passes, so each of the ~1 000 bins per
 *   column and each pixel's bin span compares numerically instead of calling `Number.isNaN`
 *   (`out` still gets `NaN`, unchanged);
 * - the one-frame-row and two-frame-row cases — every zoom level the app actually draws at — read
 *   their tile row through hoisted locals instead of indexing the `rowData`/`rowBase`/`rowBins`
 *   arrays per bin, and `dequantizeDb` is inlined as the same `-150 + code · 156 / 255`
 *   expression (identical float operations, so the values are bit-for-bit the old ones — the
 *   `columnDb` ≡ `pixelDb` parity test in `samplerColumn.test.ts` is exact).
 */
export function columnDb(
  lookup: TileLookup,
  totalFrames: number,
  bins: number,
  frameLo: number,
  frameHi: number,
  binLo: ArrayLike<number>,
  binHi: ArrayLike<number>,
  out: Float64Array,
  scratch: ColumnScratch,
): void {
  // Time axis: `sampleGridSpan(totalFrames, frameLo, frameHi, …)`'s frame choice, shared by all bins.
  let rows = 0;
  let interpolate = false;
  let frac = 0;
  if (totalFrames > 0 && frameHi > frameLo) {
    const clampedLo = Math.max(0, frameLo);
    const clampedHi = Math.min(totalFrames, frameHi);
    if (clampedHi > clampedLo) {
      let f0: number;
      let f1: number;
      if (clampedHi - clampedLo >= 1) {
        f0 = Math.max(0, Math.floor(clampedLo));
        f1 = Math.min(totalFrames - 1, Math.ceil(clampedHi) - 1);
      } else {
        interpolate = true;
        const center = Math.min(totalFrames - 1, Math.max(0, (frameLo + frameHi) / 2));
        f0 = Math.floor(center);
        f1 = Math.min(totalFrames - 1, f0 + 1);
        frac = center - f0;
      }
      let lastIndex = -1;
      let lastTile: SpectroTile | undefined;
      const count = interpolate ? 2 : f1 - f0 + 1;
      for (let k = 0; k < count; k++) {
        const frame = interpolate ? (k === 0 ? f0 : f1) : f0 + k;
        const tileIndex = Math.floor(frame / TILE_FRAMES);
        if (tileIndex !== lastIndex) {
          lastIndex = tileIndex;
          lastTile = lookup(tileIndex);
        }
        const local = frame - tileIndex * TILE_FRAMES;
        if (lastTile && local < lastTile.frames) {
          scratch.rowData[k] = lastTile.data;
          scratch.rowBase[k] = local * lastTile.bins;
          scratch.rowBins[k] = lastTile.bins;
        } else {
          scratch.rowData[k] = null;
          scratch.rowBase[k] = 0;
          scratch.rowBins[k] = 0;
        }
      }
      rows = count;
    }
  }

  // The bins any row can touch (a row interpolating reads its centre bin and the next one).
  let minLo = Infinity;
  let maxHi = -Infinity;
  for (let py = 0; py < out.length; py++) {
    minLo = Math.min(minLo, binLo[py]!);
    maxHi = Math.max(maxHi, binHi[py]!);
  }
  const firstBin = Math.max(0, Math.floor(minLo));
  const lastBin = Math.min(bins - 1, Math.ceil(maxHi) + 1);
  const values = scratch.values;
  const { rowData, rowBase, rowBins } = scratch;
  if (rows === 0) {
    values.fill(UNKNOWN, firstBin, lastBin + 1);
  } else if (interpolate) {
    // Two frame rows, linearly interpolated (zoomed in past one frame per pixel).
    const d0 = rowData[0] ?? null;
    const d1 = rowData[1] ?? null;
    const b0 = rowBase[0]!;
    const b1 = rowBase[1]!;
    const n0 = d0 ? rowBins[0]! : 0;
    const n1 = d1 ? rowBins[1]! : 0;
    for (let bin = firstBin; bin <= lastBin; bin++) {
      const has0 = bin < n0;
      const has1 = bin < n1;
      if (has0 && has1) {
        const v0 = CODE_DB[d0![b0 + bin]!]!;
        const v1 = CODE_DB[d1![b1 + bin]!]!;
        values[bin] = v0 + (v1 - v0) * frac;
      } else if (has0) {
        values[bin] = CODE_DB[d0![b0 + bin]!]!;
      } else if (has1) {
        values[bin] = CODE_DB[d1![b1 + bin]!]!;
      } else {
        values[bin] = UNKNOWN;
      }
    }
  } else if (rows === 1) {
    // One frame row (the zoomed-out case: one or two frames per device column).
    const d = rowData[0] ?? null;
    if (!d) {
      values.fill(UNKNOWN, firstBin, lastBin + 1);
    } else {
      const base = rowBase[0]!;
      const top = Math.min(lastBin, rowBins[0]! - 1);
      for (let bin = firstBin; bin <= top; bin++) {
        values[bin] = CODE_DB[d[base + bin]!]!;
      }
      values.fill(UNKNOWN, top + 1, lastBin + 1);
    }
  } else {
    // The maximum over three or more frame rows.
    for (let bin = firstBin; bin <= lastBin; bin++) {
      let best = -1;
      for (let k = 0; k < rows; k++) {
        const d = rowData[k];
        if (d && bin < rowBins[k]!) {
          const code = d[rowBase[k]! + bin]!;
          if (code > best) {
            best = code;
          }
        }
      }
      values[bin] = best >= 0 ? CODE_DB[best]! : UNKNOWN;
    }
  }

  // Frequency axis: `sampleGridSpan(bins, binLo[py], binHi[py], …)` over the column's values.
  for (let py = 0; py < out.length; py++) {
    const lo = binLo[py]!;
    const hi = binHi[py]!;
    let v = UNKNOWN;
    if (bins > 0 && hi > lo) {
      const clampedLo = Math.max(0, lo);
      const clampedHi = Math.min(bins, hi);
      if (clampedHi > clampedLo) {
        if (clampedHi - clampedLo >= 1) {
          const i0 = Math.max(0, Math.floor(clampedLo));
          const i1 = Math.min(bins - 1, Math.ceil(clampedHi) - 1);
          for (let i = i0; i <= i1; i++) {
            const x = values[i]!;
            if (x > v) {
              v = x;
            }
          }
        } else {
          const center = Math.min(bins - 1, Math.max(0, (lo + hi) / 2));
          const i0 = Math.floor(center);
          const i1 = Math.min(bins - 1, i0 + 1);
          const v0 = values[i0]!;
          const v1 = values[i1]!;
          v = v0 === UNKNOWN ? v1 : v1 === UNKNOWN ? v0 : v0 + (v1 - v0) * (center - i0);
        }
      }
    }
    out[py] = v === UNKNOWN ? Number.NaN : v;
  }
}

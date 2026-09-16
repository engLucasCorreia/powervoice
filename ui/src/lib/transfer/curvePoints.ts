/**
 * Turns a `TransferCurveDto` into canvas polylines (H-63). Pure and canvas-free so the geometry
 * is testable in jsdom; no level maths of its own — every dB value comes from Rust's
 * `TransferCurve` (SPEC-016 §4.11).
 */

import type { TransferCurveDto } from "../ipc/bindings";
import { TRANSFER_MIN_DBFS, xForLevel, yForLevel } from "./levelAxis";

export interface ScreenPoint {
  x: number;
  y: number;
}

/** Below this the output is off the bottom of the graph: a muted level (Rust's −∞, reported as
 * `min_dbfs` because JSON has no −∞) breaks the polyline instead of diving to the corner. */
function isDrawable(db: number, minDbfs: number): boolean {
  return Number.isFinite(db) && db > Math.max(minDbfs, TRANSFER_MIN_DBFS - 1e-9);
}

/**
 * One branch as canvas points, `null` wherever the output is muted or non-finite. Callers start a
 * new sub-path at each `null`, so a gate's silent region leaves a gap rather than a spike.
 */
export function branchToScreen(
  curve: TransferCurveDto,
  outDb: readonly number[],
  width: number,
  height: number,
): (ScreenPoint | null)[] {
  return curve.in_dbfs.map((inDb, i) => {
    const out = outDb[i];
    if (out === undefined || !Number.isFinite(inDb) || !isDrawable(out, curve.min_dbfs)) {
      return null;
    }
    return { x: xForLevel(inDb, width), y: yForLevel(out, height) };
  });
}

/**
 * Index ranges `[start, end]` (inclusive) where the Falling branch differs from Rising by more
 * than `epsilonDb` — the hysteresis loop, the only part drawn dashed (SPEC-016 §2.6). Ranges are
 * widened by one sample on each side so the dashed segment meets the solid curve.
 */
export function differingRanges(
  rising: readonly number[],
  falling: readonly number[],
  epsilonDb = 1e-6,
): Array<[number, number]> {
  const n = Math.min(rising.length, falling.length);
  const ranges: Array<[number, number]> = [];
  let start: number | null = null;
  for (let i = 0; i < n; i += 1) {
    const r = rising[i] ?? 0;
    const f = falling[i] ?? 0;
    const differs = r !== f && !(Math.abs(r - f) <= epsilonDb);
    if (differs && start === null) {
      start = i;
    } else if (!differs && start !== null) {
      ranges.push([Math.max(0, start - 1), Math.min(n - 1, i)]);
      start = null;
    }
  }
  if (start !== null) {
    ranges.push([Math.max(0, start - 1), n - 1]);
  }
  return ranges;
}

/** Points of one branch between two indices (inclusive), `null`s kept. */
export function sliceScreen(
  points: (ScreenPoint | null)[],
  [start, end]: [number, number],
): (ScreenPoint | null)[] {
  return points.slice(start, end + 1);
}

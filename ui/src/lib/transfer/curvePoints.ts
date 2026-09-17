/**
 * Turns a decoded `VXTC` frame into canvas polylines (H-63, H-77). Pure and canvas-free so the
 * geometry is testable in jsdom; no level maths of its own — every dB value comes from Rust's
 * `TransferCurve` (SPEC-016 §4.11).
 */

import type { TransferCurveFrame } from "../ipc/transferCurve";
import { xForLevel, yForLevel } from "./levelAxis";

export interface ScreenPoint {
  x: number;
  y: number;
}

/** A muted level (Rust's −∞, which `VXTC` carries as an `f32` −∞) breaks the polyline instead of
 * diving to the corner. */
function isDrawable(db: number): boolean {
  return Number.isFinite(db);
}

/**
 * One branch as canvas points, `null` wherever the output is muted or non-finite. Callers start a
 * new sub-path at each `null`, so a gate's silent region leaves a gap rather than a spike.
 */
export function branchToScreen(
  curve: TransferCurveFrame,
  outDb: readonly number[],
  width: number,
  height: number,
): (ScreenPoint | null)[] {
  return curve.inDbfs.map((inDb, i) => {
    const out = outDb[i];
    if (out === undefined || !Number.isFinite(inDb) || !isDrawable(out)) {
      return null;
    }
    return { x: xForLevel(inDb, width), y: yForLevel(out, height) };
  });
}

/**
 * One component's own contribution as canvas points (H-77, SPEC-016 §2.6 "per-component curve
 * overlays"): the output the section alone would produce, `input + its gain`, so the overlay
 * reads on the same axes as the total curve. `null` wherever the section mutes.
 */
export function componentToScreen(
  curve: TransferCurveFrame,
  gainDb: readonly number[],
  width: number,
  height: number,
): (ScreenPoint | null)[] {
  return curve.inDbfs.map((inDb, i) => {
    const gain = gainDb[i];
    if (gain === undefined || !Number.isFinite(inDb) || !isDrawable(gain)) {
      return null;
    }
    return { x: xForLevel(inDb, width), y: yForLevel(inDb + gain, height) };
  });
}

/** True when a component does something at some level (a disabled section contributes exactly
 * 0 dB everywhere, and drawing that on the 1:1 diagonal is only noise). */
export function componentIsActive(gainDb: readonly number[], epsilonDb = 1e-6): boolean {
  return gainDb.some((g) => !Number.isFinite(g) || Math.abs(g) > epsilonDb);
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

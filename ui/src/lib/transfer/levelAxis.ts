/**
 * The transfer graph's level axis (H-63, SPEC-016 §2.6): input peak level of a steady sine
 * against output peak level, the **same** linear dBFS scale on both axes, −80 … +6 dBFS, so the
 * 1:1 line is the square's diagonal. Pure — no DSP here; the curve itself always comes from
 * Rust's `TransferCurve` (SPEC-016 §4.11).
 */

import { formatNumber } from "../ui/units";

/** Axis minimum (SPEC-016 §2.6 "−80 … +6 dBFS"). */
export const TRANSFER_MIN_DBFS = -80;
/** Axis maximum. */
export const TRANSFER_MAX_DBFS = 6;
/** Grid line every 6 dB (SPEC-016 §2.6). */
export const TRANSFER_GRID_STEP_DB = 6;
/** Labelled every 12 dB. */
export const TRANSFER_LABEL_STEP_DB = 12;
/** Graph side, square and following the panel width (SPEC-016 §2.6 "square, 200–320 px"). */
export const TRANSFER_MIN_SIDE_PX = 200;
export const TRANSFER_MAX_SIDE_PX = 320;
/** `vox_engine::MAX_TRANSFER_CURVE_POINTS`; the backend clamps to it as well. */
export const TRANSFER_MAX_POINTS = 1024;

const SPAN_DB = TRANSFER_MAX_DBFS - TRANSFER_MIN_DBFS;

/** The graph's side for a panel `width`, clamped to the spec's range. */
export function transferSidePx(width: number): number {
  if (!(width > 0)) {
    return TRANSFER_MIN_SIDE_PX;
  }
  return Math.round(Math.min(TRANSFER_MAX_SIDE_PX, Math.max(TRANSFER_MIN_SIDE_PX, width)));
}

/** Pixel x (0…width) for an input level, clamped to the axis. */
export function xForLevel(db: number, width: number): number {
  const clamped = Math.min(Math.max(db, TRANSFER_MIN_DBFS), TRANSFER_MAX_DBFS);
  return ((clamped - TRANSFER_MIN_DBFS) / SPAN_DB) * width;
}

/** Inverse of {@link xForLevel}, clamped to the axis. */
export function levelForX(x: number, width: number): number {
  if (!(width > 0)) {
    return TRANSFER_MIN_DBFS;
  }
  const db = TRANSFER_MIN_DBFS + (x / width) * SPAN_DB;
  return Math.min(Math.max(db, TRANSFER_MIN_DBFS), TRANSFER_MAX_DBFS);
}

/** Pixel y (0…height) for an output level; louder is higher on screen. */
export function yForLevel(db: number, height: number): number {
  const clamped = Math.min(Math.max(db, TRANSFER_MIN_DBFS), TRANSFER_MAX_DBFS);
  return height - ((clamped - TRANSFER_MIN_DBFS) / SPAN_DB) * height;
}

export interface LevelTick {
  db: number;
  /** A labelled grid line (every 12 dB) rather than a plain one (every 6 dB). */
  major: boolean;
  label: string;
}

/** Every grid value in the axis range, anchored on 0 dBFS. */
export function transferAxisTicks(): LevelTick[] {
  const ticks: LevelTick[] = [];
  const first = Math.ceil(TRANSFER_MIN_DBFS / TRANSFER_GRID_STEP_DB) * TRANSFER_GRID_STEP_DB;
  for (let db = first; db <= TRANSFER_MAX_DBFS + 1e-9; db += TRANSFER_GRID_STEP_DB) {
    const rounded = Math.round(db);
    ticks.push({
      db: rounded,
      major: rounded % TRANSFER_LABEL_STEP_DB === 0,
      label: formatNumber(rounded, 0, { signed: rounded > 0 }),
    });
  }
  return ticks;
}

/** How many curve points to ask Rust for at this graph width: one per CSS pixel column, at least
 * two and never more than the backend's cap. */
export function transferCurvePointCount(width: number): number {
  if (!(width > 0)) {
    return 2;
  }
  return Math.min(TRANSFER_MAX_POINTS, Math.max(2, Math.round(width)));
}

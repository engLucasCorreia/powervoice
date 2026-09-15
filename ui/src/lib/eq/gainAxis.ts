/**
 * Linear-dB gain axis for the EQ graph (S3-07, SPEC-015 §2.6.2): ±12 dB by default, ±24 dB
 * option. 0 dB sits at mid-height; positive gain draws upward (smaller `y`).
 */

import { formatNumber } from "../ui/units";

/** Default gain range (SPEC-015 §2.6.2 "±12 dB by default"). */
export const EQ_GAIN_RANGE_DEFAULT_DB = 12;
/** The wider option (SPEC-015 §2.6.2 "±24 covers the whole parameter range"). */
export const EQ_GAIN_RANGE_WIDE_DB = 24;

/** Pixel y (0…height) for `db` on a linear axis of ±`rangeDb`. Clamped to the edges. */
export function yForDb(db: number, height: number, rangeDb: number): number {
  if (!(rangeDb > 0)) {
    return height / 2;
  }
  const clamped = Math.min(Math.max(db, -rangeDb), rangeDb);
  return height / 2 - (clamped / rangeDb) * (height / 2);
}

/** Inverse of {@link yForDb}: the dB value at pixel `y`, clamped to `[-rangeDb, rangeDb]`. */
export function dbForY(y: number, height: number, rangeDb: number): number {
  if (!(height > 0) || !(rangeDb > 0)) {
    return 0;
  }
  const t = (height / 2 - y) / (height / 2);
  return Math.min(Math.max(t * rangeDb, -rangeDb), rangeDb);
}

/** The grid/label step for `rangeDb` (H-24 item 8, matching `EqGraph.svelte`'s existing grid:
 * every 3 dB for the ±12 dB range, every 6 dB for the wider ±24 dB one). */
export function gainGridStepDb(rangeDb: number): number {
  return rangeDb === EQ_GAIN_RANGE_WIDE_DB ? 6 : 3;
}

export interface GainTick {
  db: number;
  /** Pixel y (0…height). */
  y: number;
  label: string;
}

/** "+12", "0", "−12" (H-24 item 8: dB labels at the standard ±12/±24 range, unit shown once by
 * the caller; H-26: the true minus sign from `ui/units.ts`). */
export function formatGainDb(db: number): string {
  return formatNumber(db, 0, { signed: true });
}

/** Every grid-line dB value for `rangeDb`, with its y and label (H-24 item 8). */
export function gainAxisTicks(height: number, rangeDb: number): GainTick[] {
  if (!(height > 0) || !(rangeDb > 0)) {
    return [];
  }
  const step = gainGridStepDb(rangeDb);
  const ticks: GainTick[] = [];
  for (let db = -rangeDb; db <= rangeDb + 1e-9; db += step) {
    const rounded = Math.round(db);
    ticks.push({ db: rounded, y: yForDb(rounded, height, rangeDb), label: formatGainDb(rounded) });
  }
  return ticks;
}

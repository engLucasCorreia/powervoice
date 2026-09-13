/**
 * Linear-dB gain axis for the EQ graph (S3-07, SPEC-015 §2.6.2): ±12 dB by default, ±24 dB
 * option. 0 dB sits at mid-height; positive gain draws upward (smaller `y`).
 */

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

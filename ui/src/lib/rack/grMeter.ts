/**
 * The gain-reduction meter's scale (H-77, SPEC-016 §2.6 "Gain-reduction meters"): a bar growing
 * leftwards from 0 dB over 0 … −30 dB with ticks at 0, −3, −6, −10, −20 and −30. Pure, so the
 * geometry is testable without a DOM.
 *
 * A channel that declares a **narrower** display range keeps it (SPEC-017 §2.3: the true-peak
 * limiter's meter is 0 … −24 dB); a wider one — Dynamics declares −60 … 0, which is its value
 * floor, not a readable scale — is clamped to −30. Values past the scale pin the bar but the
 * readout keeps them, and the channel's floor itself reads "≤ −60 dB".
 */

/** The deepest reduction the bar shows (SPEC-016 §2.6). */
export const GR_SCALE_FLOOR_DB = -30;

const TICKS_DB = [0, -3, -6, -10, -20, -30];

/** The scale's bottom for a channel whose declared minimum is `channelMin`. */
export function grScaleMin(channelMin: number): number {
  return Number.isFinite(channelMin) ? Math.max(channelMin, GR_SCALE_FLOOR_DB) : GR_SCALE_FLOOR_DB;
}

/** How much of the track the bar covers, 0 (no reduction) … 1 (pinned). */
export function grFraction(db: number, scaleMin: number, max = 0): number {
  if (!(max > scaleMin)) {
    return 0;
  }
  const clamped = Math.min(max, Math.max(scaleMin, db));
  return (max - clamped) / (max - scaleMin);
}

/** The tick positions inside the scale, as fractions of the track from 0 dB. */
export function grTicks(scaleMin: number, max = 0): Array<{ db: number; fraction: number }> {
  return TICKS_DB.filter((db) => db <= max && db >= scaleMin).map((db) => ({
    db,
    fraction: grFraction(db, scaleMin, max),
  }));
}

/** True when the value has reached the channel's own floor, which reads "≤ −60 dB". */
export function grAtFloor(db: number, channelMin: number): boolean {
  return Number.isFinite(channelMin) && db <= channelMin;
}

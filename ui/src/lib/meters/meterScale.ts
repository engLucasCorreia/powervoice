/**
 * Canvas-free math for the output meter's vertical dBFS scale (H-41): a fixed floor
 * (`METER_FLOOR_DB`), collision-free tick labels (reusing `ui/axisLabels.ts`'s fitting logic,
 * the same one the analyzer/EQ/waveform axes use) and the safe/loud/hot colour-zone boundaries.
 *
 * No function here takes a *level* — only a container height and font metrics — so the scale
 * itself can never depend on the meter's live values; only the moving bars do. That is what makes
 * "the column's width never changes with the values" true by construction rather than by care.
 */

import { formatNumber } from "../ui/units";
import { fitAxisLabels, type LabelAlign } from "../ui/axisLabels";

/** Scale floor, dBFS (matches the existing input-meter/meter-bridge convention: −60…0 dBFS). */
export const METER_FLOOR_DB = -60;

/**
 * Colour-zone boundaries (H-41 ticket: "safe, loud, and hot above −3 dBFS" — the ticket gives the
 * hot threshold explicitly; the safe/loud split is this ticket's own choice, a conventional
 * −18 dBFS "getting loud" onset).
 */
export const LOUD_ZONE_DB = -18;
export const HOT_ZONE_DB = -3;

/** The owner's explicit tick list (H-41 ticket), loudest first — the order ticks are dropped in
 * first when the meter is too short to fit them all. */
const FINITE_TICKS_DB = [0, -3, -6, -12, -18, -24, -36, -48, -60] as const;

export interface MeterTick {
  /** Pixel y from the top of the track (0 = 0 dBFS, `heightPx` = the absolute floor). */
  y: number;
  label: string;
  db: number;
  /** How the label sits relative to `y` (edge ticks anchor inward so they're never cut off by the
   * top/bottom of the track — see `ui/axisLabels.ts::edgeAlign`). */
  align: LabelAlign;
}

/**
 * 0 (at or below the floor) … 1 (at or above 0 dBFS) fraction for `db`, clamped. The single
 * source of truth the bars, the ticks and the colour zones all share, so none of them can ever
 * disagree about where a dB value sits on the scale.
 */
export function meterFraction(db: number): number {
  if (!Number.isFinite(db) || db <= METER_FLOOR_DB) {
    return 0;
  }
  if (db >= 0) {
    return 1;
  }
  return (db - METER_FLOOR_DB) / -METER_FLOOR_DB;
}

/**
 * Tick labels for the vertical scale: the owner's fixed dBFS ladder plus "−∞" pinned to the
 * absolute bottom edge. "−∞" is always kept (it's the floor of both the bar and the ruler); the
 * finite ladder is fitted into the room left above it, so the two can never collide however
 * short the meter gets — unlike the waveform's amplitude ruler, −∞ is a genuine value here (the
 * meter's silent rest position), not an unlabelable centerline, so it earns its own tick.
 */
export function meterScaleTicks(heightPx: number, lineHeightPx: number, gapPx = 2): MeterTick[] {
  if (!(heightPx > 0) || !(lineHeightPx > 0)) {
    return [];
  }
  const usablePx = Math.max(0, heightPx - lineHeightPx - gapPx);
  const finite = fitAxisLabels(
    FINITE_TICKS_DB.map((db) => ({ pos: usablePx * (1 - meterFraction(db)), size: lineHeightPx, db })),
    { length: usablePx, gapPx },
  );
  return [
    ...finite.map((f) => ({ y: f.pos, label: formatNumber(f.db, 0), db: f.db, align: f.align })),
    {
      y: heightPx,
      label: formatNumber(Number.NEGATIVE_INFINITY, 0),
      db: Number.NEGATIVE_INFINITY,
      // Anchored so the label sits just above the absolute bottom edge, never below it.
      align: "end" as const,
    },
  ];
}

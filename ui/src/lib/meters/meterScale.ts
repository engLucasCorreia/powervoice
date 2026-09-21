/**
 * Canvas-free math for a vertical dBFS meter's scale (H-41, generalized by H-112 for a selectable
 * floor): collision-free tick labels (reusing `ui/axisLabels.ts`'s fitting logic, the same one the
 * analyzer/EQ/waveform axes use) and the safe/loud/hot colour-zone boundaries.
 *
 * No function here takes a *level* — only a container height, font metrics and (H-112) a floor —
 * so the scale itself can never depend on the meter's live values; only the moving bars do. That
 * is what makes "the column's width never changes with the values" true by construction rather
 * than by care.
 */

import { formatNumber } from "../ui/units";
import { fitAxisLabels, type LabelAlign } from "../ui/axisLabels";

/** Default/output-meter scale floor, dBFS (H-41/H-48; the owner likes the output meter as-is, so
 * this ticket (H-112) never changes it — only the input meter's floor becomes selectable). */
export const METER_FLOOR_DB = -60;

/**
 * H-112 (owner request): "options to change the mic scale's minimum to −60, −80 or −120". The
 * factory default (`METER_FLOOR_DB`) stays first/loudest.
 */
export const INPUT_METER_FLOOR_CHOICES_DB = [-60, -80, -120] as const;

/**
 * Colour-zone boundaries (H-41 ticket: "safe, loud, and hot above −3 dBFS" — the ticket gives the
 * hot threshold explicitly; the safe/loud split is this ticket's own choice, a conventional
 * −18 dBFS "getting loud" onset). Fixed absolute dBFS values regardless of the scale's floor —
 * only where they *land* on the scale (their fraction) depends on the floor.
 */
export const LOUD_ZONE_DB = -18;
export const HOT_ZONE_DB = -3;

/** The owner's explicit tick list for the factory −60 dBFS floor (H-41 ticket), loudest first —
 * the order ticks are dropped in first when the meter is too short to fit them all. */
const BASE_TICKS_DB = [0, -3, -6, -12, -18, -24, -36, -48] as const;

/**
 * The tick ladder for a given floor, loudest first (same drop-order convention as
 * `BASE_TICKS_DB`). For the factory −60 dBFS floor this is byte-for-byte `BASE_TICKS_DB` plus
 * −60, i.e. the original H-41 ladder. A deeper floor (H-112: −80/−120 dBFS) extends it with
 * further rungs every 12 dB so the scale still reads cleanly all the way down, instead of leaving
 * a big unlabelled gap between −48 and the floor.
 */
function tickLadderForFloor(floorDb: number): number[] {
  const ladder: number[] = [...BASE_TICKS_DB];
  for (let db = -60; db > floorDb; db -= 12) {
    ladder.push(db);
  }
  ladder.push(floorDb);
  return ladder;
}

/**
 * Minimum clear space kept between two neighbouring tick labels (H-48 item 2 owner report:
 * "-12/-18/-24 run together" at short dock heights). `fitAxisLabels`'s own default (2 px) is only
 * enough to stop literal overlap — at some dock heights that let three ticks survive the fit only
 * 2 px apart, which reads as crowded even though nothing technically collides. A few more pixels
 * gives every kept label real breathing room, at the cost of dropping one more tick sooner as the
 * meter gets shorter — exactly the "drop labels that don't fit" the ticket asks for.
 */
export const METER_LABEL_GAP_PX = 4;

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
 * 0 (at or below `floorDb`) … 1 (at or above 0 dBFS) fraction for `db`, clamped. The single source
 * of truth the bars, the ticks and the colour zones all share, so none of them can ever disagree
 * about where a dB value sits on the scale. `floorDb` defaults to the output meter's fixed floor
 * (H-41/H-48); the input meter (H-112) passes its own selected floor instead.
 */
export function meterFraction(db: number, floorDb: number = METER_FLOOR_DB): number {
  if (!Number.isFinite(db) || db <= floorDb) {
    return 0;
  }
  if (db >= 0) {
    return 1;
  }
  return (db - floorDb) / -floorDb;
}

/**
 * Tick labels for the vertical scale: the floor's dBFS ladder (`tickLadderForFloor`) plus "−∞"
 * pinned to the absolute bottom edge. "−∞" is always kept (it's the floor of both the bar and the
 * ruler); the finite ladder is fitted into the room left above it, so the two can never collide
 * however short the meter gets — unlike the waveform's amplitude ruler, −∞ is a genuine value here
 * (the meter's silent rest position), not an unlabelable centerline, so it earns its own tick.
 *
 * `floorDb` defaults to the output meter's fixed −60 dBFS floor (H-41/H-48); the input meter
 * (H-112) passes its own selected floor (−60/−80/−120) instead — the ladder, the pixel positions
 * and the "always try to keep 0 and −∞" thinning all follow automatically.
 */
export function meterScaleTicks(
  heightPx: number,
  lineHeightPx: number,
  gapPx = METER_LABEL_GAP_PX,
  floorDb: number = METER_FLOOR_DB,
): MeterTick[] {
  if (!(heightPx > 0) || !(lineHeightPx > 0)) {
    return [];
  }
  const usablePx = Math.max(0, heightPx - lineHeightPx - gapPx);
  const finite = fitAxisLabels(
    tickLadderForFloor(floorDb).map((db) => ({ pos: usablePx * (1 - meterFraction(db, floorDb)), size: lineHeightPx, db })),
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

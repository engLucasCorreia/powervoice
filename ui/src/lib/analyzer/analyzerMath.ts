/**
 * Canvas-free math for the analyzer panel (H-16, SPEC-007 §2.9): the dB axis mapping for the
 * floor/ceiling picker, and the nearest-band lookup for the hover readout. Kept out of
 * `AnalyzerPanel.svelte` so it's testable under jsdom, which has no canvas (MEMORY.md).
 */

/** SPEC-007 §2.9 / §3 `an_floor_db`: the floor can be one of these five values. */
export const ANALYZER_FLOOR_OPTIONS_DB = [-150, -120, -100, -80, -60] as const;
/** SPEC-007 §2.9 / §3 `an_ceil_db`: the ceiling is 0 or +6. */
export const ANALYZER_CEIL_OPTIONS_DB = [0, 6] as const;

/** SPEC-007 §2.9 factory default. */
export const DEFAULT_ANALYZER_FLOOR_DB = -120;
/** SPEC-007 §2.9 factory default. */
export const DEFAULT_ANALYZER_CEIL_DB = 0;

/** Pixel y (0 = top, `heightPx` = bottom) for `db` on `[floorDb, ceilDb]` (bottom = floor,
 * clamped to the axis). */
export function yForAnalyzerDb(
  db: number,
  floorDb: number,
  ceilDb: number,
  heightPx: number,
): number {
  if (!(ceilDb > floorDb)) {
    return heightPx;
  }
  const t = (db - floorDb) / (ceilDb - floorDb);
  return (1 - Math.min(1, Math.max(0, t))) * heightPx;
}

/** Inverse of {@link yForAnalyzerDb}: the dB value at pixel row `y`, clamped to `[floorDb,
 * ceilDb]`. */
export function dbForAnalyzerY(
  y: number,
  floorDb: number,
  ceilDb: number,
  heightPx: number,
): number {
  if (!(ceilDb > floorDb) || heightPx <= 0) {
    return floorDb;
  }
  const t = 1 - Math.min(1, Math.max(0, y / heightPx));
  return floorDb + t * (ceilDb - floorDb);
}

export interface DbTick {
  /** Pixel y (0 = top, `heightPx` = bottom). */
  y: number;
  label: string;
  db: number;
}

/**
 * dB axis ticks for the analyzer panel's left gutter (H-24 item 5): every 10 dB by default, or
 * every 20 dB when the pane is too short for 10 dB spacing to avoid overlapping labels ("labels
 * never overlap (drop minor labels by available pixels)").
 */
export function dbAxisTicks(
  floorDb: number,
  ceilDb: number,
  heightPx: number,
  minLabelGapPx: number,
): DbTick[] {
  if (!(ceilDb > floorDb) || !(heightPx > 0)) {
    return [];
  }
  const span = ceilDb - floorDb;
  const stepFor = (step: number): number => (heightPx * step) / span;
  const step = stepFor(10) >= minLabelGapPx ? 10 : 20;
  const ticks: DbTick[] = [];
  const first = Math.ceil(floorDb / step) * step;
  for (let db = first; db <= ceilDb + 1e-9; db += step) {
    ticks.push({ y: yForAnalyzerDb(db, floorDb, ceilDb, heightPx), label: String(Math.round(db)), db });
  }
  return ticks;
}

/**
 * The band index whose centre `f0Hz · 2^(k / bandsPerOctave)` (SPEC-007 §4.8: `f_k = 20·2^(k/24)`)
 * is nearest `freqHz`, clamped to `[0, bandCount - 1]`. Used for the hover readout: the pointer's
 * frequency maps to the band whose *measured* level is shown, rather than a value interpolated
 * from the pointer's vertical position (which has no relation to the curve's actual height there).
 */
export function nearestAnalyzerBand(
  freqHz: number,
  f0Hz: number,
  bandsPerOctave: number,
  bandCount: number,
): number {
  if (bandCount <= 0) {
    return 0;
  }
  if (!(freqHz > 0) || !(f0Hz > 0) || bandsPerOctave <= 0) {
    return 0;
  }
  const k = Math.round(bandsPerOctave * Math.log2(freqHz / f0Hz));
  return Math.min(Math.max(k, 0), bandCount - 1);
}

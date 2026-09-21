/**
 * Pure geometry for the "Explain My Voice" graph's dB axis (H-92 ticket §3: "dBFS level axis
 * over the measured range, never truncating real peaks").
 */

const HEADROOM_DB = 6;
const MIN_SPAN_DB = 30;
const STEP_DB = 10;
const FALLBACK_FLOOR_DB = -120;
const FALLBACK_CEIL_DB = 0;

function extent(values: ArrayLike<number>): { min: number; max: number } | null {
  let min = Infinity;
  let max = -Infinity;
  for (let i = 0; i < values.length; i++) {
    const v = values[i];
    if (v !== undefined && Number.isFinite(v)) {
      if (v < min) min = v;
      if (v > max) max = v;
    }
  }
  return Number.isFinite(min) && Number.isFinite(max) ? { min, max } : null;
}

/**
 * `[floorDb, ceilDb]` that contains every finite value of both curves with
 * {@link HEADROOM_DB} dB of headroom above the loudest point (so a real peak never touches the
 * top of the plot), rounded outward to a clean {@link STEP_DB} dB grid, and at least
 * {@link MIN_SPAN_DB} dB tall. Falls back to the analyzer's usual −120…0 dBFS when neither curve
 * has a finite point (an empty snapshot).
 */
export function computeDbRange(rawDb: ArrayLike<number>, smoothedDb: ArrayLike<number>): [number, number] {
  const a = extent(rawDb);
  const b = extent(smoothedDb);
  if (!a && !b) {
    return [FALLBACK_FLOOR_DB, FALLBACK_CEIL_DB];
  }
  const min = Math.min(a?.min ?? Infinity, b?.min ?? Infinity);
  const max = Math.max(a?.max ?? -Infinity, b?.max ?? -Infinity);
  let ceilDb = Math.ceil((max + HEADROOM_DB) / STEP_DB) * STEP_DB;
  let floorDb = Math.floor((min - HEADROOM_DB) / STEP_DB) * STEP_DB;
  if (ceilDb - floorDb < MIN_SPAN_DB) {
    floorDb = ceilDb - MIN_SPAN_DB;
  }
  return [floorDb, ceilDb];
}

/**
 * Pure geometry for the "Explain My Voice" graph's dB axis (H-92 ticket §3: "dBFS level axis
 * over the measured range, never truncating real peaks") and for the graph's own chrome — the
 * band-label row, the F0 line, the harmonic markers and the strongest-peak marker — as hard
 * `reserved` rects for H-93's annotation layout (H-102: "check what is being passed"; before this
 * none of the graph's own markers were reserved, so a floating finding card could land right on
 * top of one).
 */
import type { Rect } from "../../ui/axisLabels";
import { freqForU } from "../../spectrum/freqAxis";

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

const F0_LABEL_W_PX = 30;
const F0_LABEL_H_PX = 16;
const HARMONIC_LABEL_W_PX = 30;
const HARMONIC_LABEL_H_PX = 20;
const STRONGEST_LABEL_W_PX = 96;
const STRONGEST_LABEL_H_PX = 20;
/** Height of the band-name row (Rumble / Fundamental / … ) drawn across the plot's top. */
export const BAND_LABEL_ROW_H_PX = 14;

/**
 * The top of the F0 dashed line's own "F0" label: at the very top of the plot, unless the
 * band-name row is drawn there too, in which case it sits just below it. Before this (H-102), the
 * two were drawn at the same fixed `plot.y + 10` regardless of each other and overlapped
 * illegibly whenever a band happened to be named "Fundamental" right where F0 was — which, for a
 * voice, is always. `draw()` and {@link markerReservedRects} both call this, so the drawn label
 * and its reservation can never drift apart.
 */
export function f0LabelTopPx(plot: Rect, showBandLabels: boolean): number {
  return plot.y + (showBandLabels ? BAND_LABEL_ROW_H_PX + 2 : 0);
}

export interface MarkerReservationInput {
  /** The plot rect, in the same plot-pixel space as every position below. */
  plot: Rect;
  /** `xForFreq(fundamentalHz)`, or `null` when there is no pitch profile to draw a line for. */
  f0X: number | null;
  /** One `{x, y}` per drawn (status "supported") harmonic marker, in plot px. */
  harmonics: readonly { x: number; y: number }[];
  /** The strongest-partial marker's position, or `null` when it isn't drawn (it is the
   * fundamental, or there is no snapshot peak). */
  strongestPeak: { x: number; y: number } | null;
  /** Whether the band-name row (Rumble / Fundamental / … ) is drawn across the plot's top. */
  showBandLabels: boolean;
}

/**
 * Hard `reserved` rects for every piece of chrome the graph itself draws on top of the curve: an
 * annotation card must never cover one of these, the same way it must never cover the legend or
 * the hover readout. Anchors are plot-pixel positions the caller already computed with
 * `xForFreq`/`yForDb` — this module only sizes the box around each one.
 */
export function markerReservedRects(input: MarkerReservationInput): Rect[] {
  const rects: Rect[] = [];
  if (input.showBandLabels) {
    rects.push({ x: input.plot.x, y: input.plot.y, width: input.plot.width, height: BAND_LABEL_ROW_H_PX });
  }
  if (input.f0X !== null) {
    rects.push({
      x: input.f0X - F0_LABEL_W_PX / 2,
      y: f0LabelTopPx(input.plot, input.showBandLabels),
      width: F0_LABEL_W_PX,
      height: F0_LABEL_H_PX,
    });
  }
  for (const h of input.harmonics) {
    rects.push({
      x: h.x - HARMONIC_LABEL_W_PX / 2,
      y: h.y - HARMONIC_LABEL_H_PX,
      width: HARMONIC_LABEL_W_PX,
      height: HARMONIC_LABEL_H_PX,
    });
  }
  if (input.strongestPeak) {
    rects.push({
      x: input.strongestPeak.x - STRONGEST_LABEL_W_PX / 2,
      y: input.strongestPeak.y - STRONGEST_LABEL_H_PX,
      width: STRONGEST_LABEL_W_PX,
      height: STRONGEST_LABEL_H_PX,
    });
  }
  return rects;
}

/** Cap matching `vox_engine::rack_api::MAX_RESPONSE_CURVE_POINTS` — the backend truncates an
 * oversized request rather than rejecting it, so a caller must cap first to keep the returned
 * points index-aligned with what it asked for. The overlay only draws a smooth line, not
 * pixel-exact node positions (unlike the EQ graph itself), so far fewer than the cap is plenty. */
export const EQ_ADVICE_MAX_POINTS = 256;

/**
 * Log-spaced frequency points to request the EQ-suggestion preview curve at (H-101): one per
 * device-pixel column centre, the same spacing `eq/freqAxis.ts::logSpacedFreqs` uses for the
 * real EQ graph (SPEC-015 §4.10), capped at {@link EQ_ADVICE_MAX_POINTS}.
 */
export function eqAdviceRequestFreqs(fLo: number, fHi: number, columns: number): number[] {
  const n = Math.max(0, Math.min(Math.round(columns), EQ_ADVICE_MAX_POINTS));
  if (n <= 0 || !(fHi > fLo)) {
    return [];
  }
  const out: number[] = new Array(n);
  for (let i = 0; i < n; i++) {
    out[i] = freqForU((i + 0.5) / n, fLo, fHi, "log");
  }
  return out;
}

/**
 * The suggested-EQ overlay's drawn levels (H-101): the measured envelope plus the previewed
 * filter's own response, at the preview curve's own frequencies — "a dashed suggestion curve
 * drawn over — never modifying — the measured spectrum" (H-92/H-94's tickets). `levelAt` is the
 * caller's own interpolation over the measured curve (`analyzer/plotGeometry.ts::levelAt`), so
 * this stays a pure combine with no curve-fitting logic of its own. A point where either input
 * is non-finite comes back `NaN`, so the caller's polyline breaks there instead of drawing a
 * wrong value.
 */
export function eqAdviceLevels(
  freqsHz: readonly number[],
  totalDb: readonly number[],
  levelAt: (freqHz: number) => number,
): number[] {
  return freqsHz.map((f, i) => {
    const base = levelAt(f);
    const delta = totalDb[i];
    return Number.isFinite(base) && delta !== undefined && Number.isFinite(delta) ? base + delta : NaN;
  });
}

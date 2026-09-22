import type { ResponseCurveDto } from "../ipc/bindings";
import { xForFreq } from "./freqAxis";
import { yForDb } from "./gainAxis";

/** One point of the drawn polyline, in CSS-pixel canvas coordinates. */
export interface ScreenPoint {
  x: number;
  y: number;
}

/**
 * Maps a `ResponseCurveDto`'s total response onto canvas pixels — the only place the EQ graph
 * turns Rust's numbers into a drawable polyline (SPEC-015 §2.6.3 / AC-17: "the UI never evaluates
 * a filter; it only maps (frequency, dB) pairs to pixels"). Pulled out of `EqGraph.svelte` so it
 * is testable with a synthetic curve, without a canvas.
 */
export function totalCurveToScreen(
  curve: ResponseCurveDto,
  width: number,
  height: number,
  fLo: number,
  fHi: number,
  rangeDb: number,
): ScreenPoint[] {
  return curve.freqs_hz.map((f, i) => ({
    x: xForFreq(f, width, fLo, fHi),
    y: yForDb(curve.total_db[i] ?? 0, height, rangeDb),
  }));
}

/**
 * The total response at an arbitrary frequency (H-111, SPEC-015 §2.6.4 "the cursor readout ...
 * linearly interpolated between the returned curve points"): a plain lerp between the two
 * `curve.freqs_hz` entries bracketing `freqHz` — `curve.freqs_hz` is always ascending
 * (`curveRequestFreqs` sorts it before it's ever requested, and Rust echoes the same order back).
 * `null` for an empty curve; clamped to the first/last point outside the curve's own range (no
 * extrapolation — same as `curve.freqs_hz`'s own edges, which already cover `[fLo, fHi]`).
 */
export function totalDbAtFreq(curve: ResponseCurveDto, freqHz: number): number | null {
  const freqs = curve.freqs_hz;
  const dbs = curve.total_db;
  const n = freqs.length;
  if (n === 0) {
    return null;
  }
  if (n === 1 || freqHz <= (freqs[0] ?? 0)) {
    return dbs[0] ?? null;
  }
  if (freqHz >= (freqs[n - 1] ?? 0)) {
    return dbs[n - 1] ?? null;
  }
  let lo = 0;
  let hi = n - 1;
  while (hi - lo > 1) {
    const mid = (lo + hi) >> 1;
    if ((freqs[mid] ?? 0) < freqHz) {
      lo = mid;
    } else {
      hi = mid;
    }
  }
  const f0 = freqs[lo] ?? 0;
  const f1 = freqs[hi] ?? 0;
  const d0 = dbs[lo] ?? 0;
  const d1 = dbs[hi] ?? 0;
  if (f1 === f0) {
    return d0;
  }
  const t = (freqHz - f0) / (f1 - f0);
  return d0 + t * (d1 - d0);
}

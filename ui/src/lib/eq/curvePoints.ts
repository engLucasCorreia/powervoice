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

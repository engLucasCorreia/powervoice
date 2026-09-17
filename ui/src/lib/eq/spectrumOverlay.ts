/**
 * Live spectrum overlay math for the EQ graph (H-84, SPEC-015 §2.6.3 item 1 / §2.6.2 "Spectrum
 * scale" / AC-21): a fixed −90…0 dBFS y scale, independent of the gain range, and the analyzer's
 * band levels reduced to screen points through the *same* `xForFreq` the total curve and nodes
 * use — so a band centre and a curve point at the same frequency always land at the same x
 * (AC-21), without this file needing to know anything about log-frequency math itself.
 */

import { columnize, type PlotPoint } from "../analyzer/plotGeometry";

/** SPEC-015 §2.6.2 "Spectrum scale": fixed, independent of the gain axis. */
export const EQ_SPECTRUM_FLOOR_DBFS = -90;
export const EQ_SPECTRUM_CEIL_DBFS = 0;

/** Pixel y (0 = top, `heightPx` = bottom) for a level on the fixed −90…0 dBFS scale. */
export function yForSpectrumDbfs(dbfs: number, heightPx: number): number {
  const span = EQ_SPECTRUM_CEIL_DBFS - EQ_SPECTRUM_FLOOR_DBFS;
  const t = (dbfs - EQ_SPECTRUM_FLOOR_DBFS) / span;
  return (1 - Math.min(1, Math.max(0, t))) * heightPx;
}

/** The overlay's fill polyline: one point per pixel column (or per band, whichever is denser),
 * clipped to `[fLo, fHi]`, mapped through the caller's own `xForFreq` (AC-21's "same x"). */
export function spectrumOverlayPoints(
  freqsHz: ArrayLike<number>,
  levelsDb: ArrayLike<number>,
  xForFreq: (freqHz: number) => number,
  fLo: number,
  fHi: number,
): PlotPoint[] {
  return columnize(freqsHz, levelsDb, xForFreq, fLo, fHi);
}

/** A minimal shape of a decoded `VXSA` frame, for the dedup check below (kept structural so this
 * file doesn't need to import `AnalyzerFrame` just for three fields). */
export interface SpectrumLevels {
  readonly reset: boolean;
  readonly silent: boolean;
  readonly levelsDb: ArrayLike<number>;
}

/**
 * True when `b` shows exactly the same curve as `a` (H-43/H-84: "must not keep the scheduler
 * awake when the analyzer is idle" — the idle heartbeat repeats the same at-rest levels, and a
 * frame that changes nothing must not trigger a redraw or a live-region announcement). A `reset`
 * frame (device reopen or rate change) is never treated as a repeat, mirroring
 * `analyzer.svelte.ts`'s `sameCurve`.
 */
export function sameSpectrumLevels(a: SpectrumLevels | null, b: SpectrumLevels): boolean {
  if (!a || a.reset || b.reset || a.silent !== b.silent || a.levelsDb.length !== b.levelsDb.length) {
    return false;
  }
  for (let i = 0; i < b.levelsDb.length; i++) {
    if (a.levelsDb[i] !== b.levelsDb[i]) {
      return false;
    }
  }
  return true;
}

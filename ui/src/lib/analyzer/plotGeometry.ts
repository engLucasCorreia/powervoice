/**
 * Canvas-free geometry for the spectrum plot (H-42, SPEC-007 §8.1): reducing a curve of up to
 * 16 385 FFT bins to about one point per pixel column, the power-domain smoothing that keeps peak
 * labels steady, the level under the pointer, and the "nothing visible changed" check that lets
 * the plot skip redraws at rest.
 */

/** A curve the spectrum plot draws: ascending frequencies and their levels. */
export interface PlotCurve {
  readonly freqsHz: ArrayLike<number>;
  readonly levelsDb: ArrayLike<number>;
  /** `bands` = the live 1/24-octave bands (drawn point for point, T-208 look); `bins` = FFT
   * bins (reduced per pixel column). */
  readonly resolution: "bands" | "bins";
}

/** A line drawn over the filled curve: a frozen snapshot (A/B) or the room tone. */
export interface PlotOverlay {
  key: string;
  curve: PlotCurve;
  tone: "a" | "b" | "noise";
  dashed?: boolean;
}

export interface PlotPoint {
  /** Plot x, px. */
  x: number;
  /** Level, dB (`-Infinity` allowed). */
  db: number;
}

/**
 * The curve's points inside `[fLo, fHi]` (plus one on each side, so lines reach the edges),
 * mapped through `xForFreq`, keeping the loudest point of every pixel column. A curve sparser
 * than the pixels (the 246 live bands) comes back point for point.
 */
export function columnize(
  freqsHz: ArrayLike<number>,
  levelsDb: ArrayLike<number>,
  xForFreq: (freqHz: number) => number,
  fLo: number,
  fHi: number,
): PlotPoint[] {
  const n = Math.min(freqsHz.length, levelsDb.length);
  const out: PlotPoint[] = [];
  let start = 0;
  while (start < n - 1 && (freqsHz[start + 1] ?? 0) < fLo) {
    start++;
  }
  let end = n - 1;
  while (end > 0 && (freqsHz[end - 1] ?? 0) > fHi) {
    end--;
  }
  let col = Number.NaN;
  let best: PlotPoint | null = null;
  for (let i = start; i <= end; i++) {
    const x = xForFreq(freqsHz[i] ?? 0);
    if (!Number.isFinite(x)) {
      continue;
    }
    const db = levelsDb[i] ?? -Infinity;
    const c = Math.floor(x);
    if (c === col && best) {
      if (db > best.db) {
        best = { x, db };
      }
      continue;
    }
    if (best) {
      out.push(best);
    }
    col = c;
    best = { x, db };
  }
  if (best) {
    out.push(best);
  }
  return out;
}

/** EMA coefficient for a step of `dtS` seconds at time constant `tauS`. */
export function emaAlpha(dtS: number, tauS: number): number {
  if (!(tauS > 0)) {
    return 1;
  }
  return 1 - Math.exp(-Math.max(0, dtS) / tauS);
}

/** Power-domain exponential smoothing of a level curve (dB in, dB out); a size change or a
 * missing `prev` restarts from `next`. */
export function smoothLevels(
  prev: Float32Array | null,
  next: ArrayLike<number>,
  alpha: number,
): Float32Array {
  const out = new Float32Array(next.length);
  const restart = !prev || prev.length !== next.length;
  for (let i = 0; i < next.length; i++) {
    const v = next[i] ?? -Infinity;
    if (restart) {
      out[i] = v;
      continue;
    }
    const pNext = Number.isFinite(v) ? 10 ** (v / 10) : 0;
    const old = prev[i] ?? -Infinity;
    const pOld = Number.isFinite(old) ? 10 ** (old / 10) : 0;
    const p = pOld + alpha * (pNext - pOld);
    out[i] = p > 0 ? 10 * Math.log10(p) : -Infinity;
  }
  return out;
}

/** Index of the point nearest `freqHz` (ascending `freqsHz`), or −1 for an empty curve. */
export function nearestIndex(freqsHz: ArrayLike<number>, freqHz: number): number {
  const n = freqsHz.length;
  if (n === 0) {
    return -1;
  }
  let lo = 0;
  let hi = n - 1;
  while (lo < hi) {
    const mid = (lo + hi) >> 1;
    if ((freqsHz[mid] ?? 0) < freqHz) {
      lo = mid + 1;
    } else {
      hi = mid;
    }
  }
  if (lo > 0 && Math.abs((freqsHz[lo - 1] ?? 0) - freqHz) <= Math.abs((freqsHz[lo] ?? 0) - freqHz)) {
    return lo - 1;
  }
  return lo;
}

/** The curve's level at the point nearest `freqHz` (`-Infinity` for an empty curve). */
export function levelAt(curve: { freqsHz: ArrayLike<number>; levelsDb: ArrayLike<number> }, freqHz: number): number {
  const i = nearestIndex(curve.freqsHz, freqHz);
  return i < 0 ? -Infinity : (curve.levelsDb[i] ?? -Infinity);
}

/** True when every level is below `floorDb` — nothing of the curve is visible. */
export function allBelow(levelsDb: ArrayLike<number>, floorDb: number): boolean {
  for (let i = 0; i < levelsDb.length; i++) {
    if ((levelsDb[i] ?? -Infinity) >= floorDb) {
      return false;
    }
  }
  return true;
}

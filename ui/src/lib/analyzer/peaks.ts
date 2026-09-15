/**
 * Spectral peak picking for the analyzer's peak labels (H-42, SPEC-007 §8.2). Works on any
 * displayed curve — the live 1/24-octave bands, a long-term average, a frozen snapshot, or the
 * Spectrum Inspector's FFT bins — given each point's frequency and level.
 *
 * A peak is a local maximum that:
 * - lies inside `[fMinHz, fMaxHz]` and at or above `floorDb`;
 * - stands at least `minProminenceDb` above the higher of the two lowest points within
 *   ±`prominenceWindowOct` on either side (a windowed topographic prominence);
 * - is at least `minSpacingOct` away from every louder peak already kept.
 * The loudest `count` such peaks are returned, loudest first, with their frequency and level
 * refined by parabolic interpolation over the three points around the maximum.
 *
 * Candidates are evaluated loudest first and the search stops once `count` are kept, so the
 * cost stays small even on a 16 385-bin curve.
 */

export interface SpectrumCurveLike {
  /** Ascending frequencies (Hz). */
  readonly freqsHz: ArrayLike<number>;
  /** Levels (dB, `-Infinity` allowed). */
  readonly levelsDb: ArrayLike<number>;
}

export interface SpectralPeak {
  /** Index of the maximum point. */
  index: number;
  /** Interpolated frequency (Hz). */
  freqHz: number;
  /** Interpolated level (dB). */
  levelDb: number;
  /** Windowed prominence (dB). */
  prominenceDb: number;
}

export interface PeakOptions {
  /** How many peaks (default 5). */
  count?: number;
  /** Minimum spacing between kept peaks, octaves (default 1/6). */
  minSpacingOct?: number;
  /** Minimum prominence, dB (default 6). */
  minProminenceDb?: number;
  /** Half-width of the prominence window, octaves (default 1/3). */
  prominenceWindowOct?: number;
  /** Ignore anything below this level (default −∞). */
  floorDb?: number;
  fMinHz?: number;
  fMaxHz?: number;
}

export const DEFAULT_PEAK_COUNT = 5;
export const DEFAULT_PEAK_SPACING_OCT = 1 / 6;
export const DEFAULT_PEAK_PROMINENCE_DB = 6;
export const DEFAULT_PROMINENCE_WINDOW_OCT = 1 / 3;

function lowerBound(freqs: ArrayLike<number>, f: number, lo: number, hi: number): number {
  let a = lo;
  let b = hi;
  while (a < b) {
    const m = (a + b) >> 1;
    if ((freqs[m] ?? 0) < f) {
      a = m + 1;
    } else {
      b = m;
    }
  }
  return a;
}

function minFinite(levels: ArrayLike<number>, from: number, to: number): number {
  let min = Infinity;
  for (let i = from; i <= to; i++) {
    const v = levels[i] ?? -Infinity;
    min = Math.min(min, Number.isFinite(v) ? v : -Infinity);
  }
  return min;
}

/** Parabolic refinement around `i` → fractional offset δ ∈ [−0.5, 0.5] and the vertex level. */
function refine(levels: ArrayLike<number>, i: number): { delta: number; level: number } {
  const a = levels[i - 1] ?? -Infinity;
  const b = levels[i] ?? -Infinity;
  const c = levels[i + 1] ?? -Infinity;
  const denom = a - 2 * b + c;
  if (!Number.isFinite(a) || !Number.isFinite(c) || !(Math.abs(denom) > 1e-12)) {
    return { delta: 0, level: b };
  }
  const delta = Math.max(-0.5, Math.min(0.5, (0.5 * (a - c)) / denom));
  return { delta, level: b - 0.25 * (a - c) * delta };
}

function interpolatedFreq(freqs: ArrayLike<number>, i: number, delta: number): number {
  const f = freqs[i] ?? 0;
  const neighbour = delta >= 0 ? freqs[i + 1] : freqs[i - 1];
  if (neighbour === undefined || !(neighbour > 0) || !(f > 0)) {
    return f;
  }
  // Geometric interpolation: right for log-spaced bands, indistinguishable from linear for
  // closely spaced FFT bins.
  return f * (neighbour / f) ** Math.abs(delta);
}

/** The loudest well-separated peaks of `curve`, loudest first. */
export function findPeaks(curve: SpectrumCurveLike, options: PeakOptions = {}): SpectralPeak[] {
  const count = options.count ?? DEFAULT_PEAK_COUNT;
  const spacing = options.minSpacingOct ?? DEFAULT_PEAK_SPACING_OCT;
  const minProminence = options.minProminenceDb ?? DEFAULT_PEAK_PROMINENCE_DB;
  const windowOct = options.prominenceWindowOct ?? DEFAULT_PROMINENCE_WINDOW_OCT;
  const floor = options.floorDb ?? -Infinity;
  const fMin = options.fMinHz ?? 0;
  const fMax = options.fMaxHz ?? Infinity;
  const { freqsHz: freqs, levelsDb: levels } = curve;
  const n = Math.min(freqs.length, levels.length);
  if (count <= 0 || n < 3) {
    return [];
  }

  const candidates: number[] = [];
  for (let i = 1; i < n - 1; i++) {
    const v = levels[i] ?? -Infinity;
    const f = freqs[i] ?? 0;
    if (!Number.isFinite(v) || v < floor || f < fMin || f > fMax || !(f > 0)) {
      continue;
    }
    if (v > (levels[i - 1] ?? -Infinity) && v >= (levels[i + 1] ?? -Infinity)) {
      candidates.push(i);
    }
  }
  candidates.sort((a, b) => (levels[b] ?? 0) - (levels[a] ?? 0));

  const kept: SpectralPeak[] = [];
  const windowRatio = 2 ** windowOct;
  for (const i of candidates) {
    if (kept.length >= count) {
      break;
    }
    const f = freqs[i] ?? 0;
    if (kept.some((p) => Math.abs(Math.log2(f / p.freqHz)) < spacing)) {
      continue;
    }
    const left = Math.max(0, lowerBound(freqs, f / windowRatio, 0, i) - 0);
    const right = Math.min(n - 1, lowerBound(freqs, f * windowRatio, i, n));
    const leftMin = left < i ? minFinite(levels, left, i - 1) : -Infinity;
    const rightMin = right > i ? minFinite(levels, i + 1, right) : -Infinity;
    const v = levels[i] ?? -Infinity;
    const prominence = v - Math.max(leftMin, rightMin);
    if (!(prominence >= minProminence)) {
      continue;
    }
    const { delta, level } = refine(levels, i);
    const freqHz = interpolatedFreq(freqs, i, delta);
    if (kept.some((p) => Math.abs(Math.log2(freqHz / p.freqHz)) < spacing)) {
      continue;
    }
    kept.push({ index: i, freqHz, levelDb: level, prominenceDb: prominence });
  }
  return kept;
}

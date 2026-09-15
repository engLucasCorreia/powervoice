/**
 * Fractional-octave smoothing for the Spectrum Inspector (H-42, SPEC-007 §8.3): each point
 * becomes the **power** mean of every point within ±1/(2n) octave of it (a 1/n-octave wide
 * rectangular window on a log axis), the usual way measurement tools smooth a spectrum. O(N)
 * with a prefix sum and two moving edges; the input must have ascending frequencies.
 */
import type { SpectrumSmoothingPref } from "../ipc/bindings";

/** Window width in octaves, or `null` for no smoothing. */
export function smoothingOctaves(pref: SpectrumSmoothingPref): number | null {
  switch (pref) {
    case "third":
      return 1 / 3;
    case "sixth":
      return 1 / 6;
    case "twelfth":
      return 1 / 12;
    default:
      return null;
  }
}

/** Smooths `levelsDb` (dB, `-Infinity` allowed) over `widthOct` octaves; points at 0 Hz are
 * copied unchanged. Returns a new array. */
export function smoothFractionalOctave(
  freqsHz: ArrayLike<number>,
  levelsDb: ArrayLike<number>,
  widthOct: number,
): Float32Array {
  const n = Math.min(freqsHz.length, levelsDb.length);
  const out = new Float32Array(n);
  const prefix = new Float64Array(n + 1);
  for (let i = 0; i < n; i++) {
    const v = levelsDb[i] ?? -Infinity;
    prefix[i + 1] = (prefix[i] ?? 0) + (Number.isFinite(v) ? 10 ** (v / 10) : 0);
  }
  const half = 2 ** (widthOct / 2);
  let lo = 0;
  let hi = 0;
  for (let i = 0; i < n; i++) {
    const f = freqsHz[i] ?? 0;
    if (!(f > 0) || !(widthOct > 0)) {
      out[i] = levelsDb[i] ?? -Infinity;
      continue;
    }
    const fLo = f / half;
    const fHi = f * half;
    while (lo < n && (freqsHz[lo] ?? 0) < fLo) {
      lo++;
    }
    if (hi < i + 1) {
      hi = i + 1;
    }
    while (hi < n && (freqsHz[hi] ?? 0) <= fHi) {
      hi++;
    }
    const start = Math.min(lo, i);
    const mean = ((prefix[hi] ?? 0) - (prefix[start] ?? 0)) / (hi - start);
    out[i] = mean > 0 ? 10 * Math.log10(mean) : -Infinity;
  }
  return out;
}

/**
 * Frequency axis mapping for the spectral pane's ruler (T-207, SPEC-007 §2.4, §4.7):
 * pixel ↔ Hz, on a **log** (20 Hz → Nyquist) or **linear** (0 Hz → Nyquist) scale, plus the
 * frequency-zoom/pan math for the ruler (§2.4) and its 1-2-5 tick ladder.
 *
 * SPEC-007 §2.10 [M4] names a single shared module `ui/src/lib/spectrum/freqAxis.ts`,
 * parameterized by `(f_lo, f_hi, scale)`, eventually reused by the analyzer panel (T-208), this
 * ruler, and the EQ graph (T-409). This file is that module's first implementation, written for
 * the spectral ruler now; unifying `ui/src/lib/eq/freqAxis.ts` (log-only, S3-07) with this one is
 * an M4 migration this ticket does not attempt (out of scope — `eq/freqAxis.ts` has its own
 * import-lint test tying it to `EqGraph.svelte` specifically).
 */

export type FreqScale = "log" | "linear";

/** Log axis bottom (SPEC-007 §2.4): log spans 20 Hz → Nyquist, linear spans 0 Hz → Nyquist. */
export const LOG_MIN_HZ = 20;

/** SPEC-007 §2.4: the minimum visible span is 1 octave (log) or 500 Hz (linear). */
export const MIN_LOG_SPAN_OCTAVES = 1;
export const MIN_LINEAR_SPAN_HZ = 500;

/** The axis's hard bottom edge for `scale` (log never goes below {@link LOG_MIN_HZ}). */
export function scaleFloorHz(scale: FreqScale): number {
  return scale === "log" ? LOG_MIN_HZ : 0;
}

/** The full range for `scale` given the document's Nyquist rate (SPEC-007 §2.4). */
export function fullFreqRange(scale: FreqScale, nyquistHz: number): [number, number] {
  const nyq = nyquistHz > 0 ? nyquistHz : 24_000;
  return [scaleFloorHz(scale), Math.max(scaleFloorHz(scale) + 1, nyq)];
}

/** Normalized height `u ∈ [0, 1]` (0 = bottom, per SPEC-007 §4.7) for `freqHz` within `[fLo,
 * fHi]`. Clamped to the edges; degenerate ranges return 0. */
export function uForFreq(freqHz: number, fLo: number, fHi: number, scale: FreqScale): number {
  if (!(fHi > fLo)) {
    return 0;
  }
  if (scale === "log") {
    const lo = Math.max(fLo, LOG_MIN_HZ);
    if (!(fHi > lo) || !(freqHz > 0)) {
      return 0;
    }
    const f = Math.min(Math.max(freqHz, lo), fHi);
    return Math.log(f / lo) / Math.log(fHi / lo);
  }
  const f = Math.min(Math.max(freqHz, fLo), fHi);
  return (f - fLo) / (fHi - fLo);
}

/** Inverse of {@link uForFreq}: the frequency at normalized height `u`, clamped to `[fLo, fHi]`. */
export function freqForU(u: number, fLo: number, fHi: number, scale: FreqScale): number {
  if (!(fHi > fLo)) {
    return fLo;
  }
  const t = Math.min(Math.max(u, 0), 1);
  if (scale === "log") {
    const lo = Math.max(fLo, LOG_MIN_HZ);
    return lo * (fHi / lo) ** t;
  }
  return fLo + t * (fHi - fLo);
}

/** Pixel y (0 = top, `heightPx` = bottom) for `freqHz` on `[fLo, fHi]` (SPEC-007 §4.7: 0 = bottom
 * of the pane in normalized terms, so higher frequency is a smaller y). */
export function yForFreq(
  freqHz: number,
  heightPx: number,
  fLo: number,
  fHi: number,
  scale: FreqScale,
): number {
  const u = uForFreq(freqHz, fLo, fHi, scale);
  return (1 - u) * heightPx;
}

/** Inverse of {@link yForFreq}: the frequency at pixel row `y`. */
export function freqForY(
  y: number,
  heightPx: number,
  fLo: number,
  fHi: number,
  scale: FreqScale,
): number {
  const u = heightPx > 0 ? 1 - y / heightPx : 0;
  return freqForU(u, fLo, fHi, scale);
}

/** Clamps a candidate `[lo, hi]` range to the axis's hard floor, the document's Nyquist rate, and
 * the minimum span (SPEC-007 §2.4), re-centring when the requested span is too narrow. */
export function clampFreqRange(
  lo: number,
  hi: number,
  scale: FreqScale,
  nyquistHz: number,
): [number, number] {
  const floor = scaleFloorHz(scale);
  const nyq = Math.max(nyquistHz, floor + 1);
  let a = Math.min(lo, hi);
  let b = Math.max(lo, hi);
  a = Math.max(a, floor);
  b = Math.min(b, nyq);
  // `a` itself may still exceed `nyq` (e.g. a pan pushed the whole range past it) — clamp it too,
  // before the degenerate-span check below, so `a` never ends up above `b`.
  a = Math.min(a, nyq);
  if (b <= a) {
    b = Math.min(nyq, a + 1);
  }
  if (scale === "log") {
    const minRatio = 2 ** MIN_LOG_SPAN_OCTAVES;
    if (b / a < minRatio) {
      const mid = Math.sqrt(a * b);
      a = mid / Math.sqrt(minRatio);
      b = mid * Math.sqrt(minRatio);
      if (a < floor) {
        b *= floor / a;
        a = floor;
      }
      if (b > nyq) {
        a *= nyq / b;
        b = nyq;
      }
      a = Math.max(a, floor);
      b = Math.min(b, nyq);
    }
  } else if (b - a < MIN_LINEAR_SPAN_HZ) {
    const mid = (a + b) / 2;
    a = mid - MIN_LINEAR_SPAN_HZ / 2;
    b = mid + MIN_LINEAR_SPAN_HZ / 2;
    if (a < floor) {
      b += floor - a;
      a = floor;
    }
    if (b > nyq) {
      a -= b - nyq;
      b = nyq;
    }
    a = Math.max(a, floor);
    b = Math.min(b, nyq);
  }
  return [a, b];
}

/** Wheel-zoom around `anchorHz` (SPEC-007 §2.4: "√2 per notch" — `factor` is the caller's step,
 * `> 1` zooms out, `< 1` zooms in), clamped to the valid range. */
export function zoomFreqRange(
  fLo: number,
  fHi: number,
  scale: FreqScale,
  anchorHz: number,
  factor: number,
  nyquistHz: number,
): [number, number] {
  let lo: number;
  let hi: number;
  if (scale === "log") {
    const a = Math.max(anchorHz, 1e-6);
    lo = a * (fLo / a) ** factor;
    hi = a * (fHi / a) ** factor;
  } else {
    lo = anchorHz + (fLo - anchorHz) * factor;
    hi = anchorHz + (fHi - anchorHz) * factor;
  }
  return clampFreqRange(lo, hi, scale, nyquistHz);
}

/** Drag-to-pan (SPEC-007 §2.4): shifts the range by `deltaFrac` of its own span (positive moves
 * toward higher frequency), clamped to the valid range. */
export function panFreqRange(
  fLo: number,
  fHi: number,
  scale: FreqScale,
  deltaFrac: number,
  nyquistHz: number,
): [number, number] {
  if (scale === "log") {
    const ratio = (fHi / fLo) ** deltaFrac;
    return clampFreqRange(fLo * ratio, fHi * ratio, scale, nyquistHz);
  }
  const shift = (fHi - fLo) * deltaFrac;
  return clampFreqRange(fLo + shift, fHi + shift, scale, nyquistHz);
}

// --- Ruler ticks (SPEC-007 §2.4) ----------------------------------------------------------------

export interface FreqTick {
  freqHz: number;
  /** Pixel y (0 = top) at the current pane height. */
  y: number;
  label: string;
}

const LADDER_MANTISSAS = [1, 2, 5] as const;

/** Every `{1, 2, 5} × 10ⁿ` Hz candidate inside `[fLo, fHi]`. */
function ladderCandidates(fLo: number, fHi: number): number[] {
  if (!(fHi > fLo) || fHi <= 0) {
    return [];
  }
  const lo = Math.max(fLo, 1e-6);
  const minExp = Math.floor(Math.log10(lo)) - 1;
  const maxExp = Math.ceil(Math.log10(fHi)) + 1;
  const out: number[] = [];
  for (let e = minExp; e <= maxExp; e++) {
    for (const m of LADDER_MANTISSAS) {
      const f = m * 10 ** e;
      if (f >= fLo && f <= fHi) {
        out.push(f);
      }
    }
  }
  return out;
}

/** "20", "500", "1k", "2.5k", "12k" (SPEC-007 §2.4: below 1000 shows plain Hz, above shows "k",
 * with a decimal only when needed). The unit "Hz" is not appended here — the ruler shows it once,
 * at the top (§2.4). */
export function formatRulerFreqHz(freqHz: number): string {
  if (freqHz < 1000) {
    return String(Math.round(freqHz));
  }
  const k = freqHz / 1000;
  const rounded = Math.round(k * 10) / 10;
  return `${Number.isInteger(rounded) ? rounded.toFixed(0) : rounded.toFixed(1)}k`;
}

/** "1 007.8 Hz" below 10 kHz, "12.35 kHz" above (SPEC-007 §2.7 hover readout format). */
export function formatHoverFreqHz(freqHz: number): string {
  if (freqHz < 10_000) {
    const value = Math.max(0, freqHz).toFixed(1);
    const [intPart = "0", decPart = "0"] = value.split(".");
    const withSep = intPart.replace(/\B(?=(\d{3})+(?!\d))/g, " ");
    return `${withSep}.${decPart} Hz`;
  }
  return `${(freqHz / 1000).toFixed(2)} kHz`;
}

/**
 * Ruler tick positions for `[fLo, fHi]` on `scale`, thinned from the 1-2-5 ladder so labels never
 * overlap at pane height `heightPx` (SPEC-007 §2.4, AC-11): candidates are walked from the top of
 * the pane (highest frequency) down, keeping one whenever it is at least `minLabelGapPx` from the
 * last kept tick.
 */
export function frequencyTicks(
  fLo: number,
  fHi: number,
  scale: FreqScale,
  heightPx: number,
  minLabelGapPx: number,
): FreqTick[] {
  if (!(fHi > fLo) || heightPx <= 0) {
    return [];
  }
  const candidates = ladderCandidates(fLo, fHi).sort((a, b) => b - a);
  const ticks: FreqTick[] = [];
  let lastY: number | null = null;
  for (const freqHz of candidates) {
    const y = yForFreq(freqHz, heightPx, fLo, fHi, scale);
    if (lastY === null || Math.abs(y - lastY) >= minLabelGapPx) {
      ticks.push({ freqHz, y, label: formatRulerFreqHz(freqHz) });
      lastY = y;
    }
  }
  return ticks.sort((a, b) => a.y - b.y);
}

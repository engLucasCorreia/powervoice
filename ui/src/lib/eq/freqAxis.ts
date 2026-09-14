/**
 * Log-frequency axis mapping for the EQ graph (S3-07, SPEC-015 §2.6.2): 20 Hz to
 * `min(20 kHz, rate / 2)`, pixel ↔ Hz.
 *
 * This is the ONLY file under `ui/src/lib/eq/` allowed to use `Math.log`/`Math.pow`/`Math.exp`
 * (AC-17's import lint, `eqGraph.lint.test.ts`, greps every other file in this directory for
 * `sin`/`cos`/`tan`/`pow`/`log`). The graph never evaluates a filter — it only maps a
 * (frequency, dB) pair Rust already computed onto pixels.
 */

/** Bottom of the graph's frequency range (SPEC-015 §2.6.2), always. */
export const EQ_MIN_HZ = 20;
/** Top of the graph's frequency range when the rate allows it (SPEC-015 §2.6.2). */
export const EQ_MAX_HZ = 20_000;

/**
 * The graph's high edge: 20 kHz, or the Nyquist rate when the document is slower (SPEC-015
 * §2.4). `rateHz <= 0` (unknown rate) falls back to 20 kHz.
 */
export function graphMaxHz(rateHz: number): number {
  if (!(rateHz > 0)) {
    return EQ_MAX_HZ;
  }
  return Math.min(EQ_MAX_HZ, rateHz / 2);
}

/** Pixel x (0…width) for `freqHz` on the log axis `[fLo, fHi]`. Clamped to the edges. */
export function xForFreq(freqHz: number, width: number, fLo: number, fHi: number): number {
  if (!(fHi > fLo) || !(freqHz > 0)) {
    return 0;
  }
  const f = Math.min(Math.max(freqHz, fLo), fHi);
  const t = Math.log(f / fLo) / Math.log(fHi / fLo);
  return t * width;
}

/** Inverse of {@link xForFreq}: the frequency at pixel `x`, clamped to `[fLo, fHi]`. */
export function freqForX(x: number, width: number, fLo: number, fHi: number): number {
  if (!(fHi > fLo)) {
    return fLo;
  }
  const t = width > 0 ? x / width : 0;
  const f = fLo * Math.pow(fHi / fLo, t);
  return Math.min(Math.max(f, fLo), fHi);
}

/** True when `freqHz` lies strictly inside `(fLo, fHi)` — otherwise a node's caret pins to the
 * edge (SPEC-015 §2.6.3 "a node outside the visible range"). */
export function inRange(freqHz: number, fLo: number, fHi: number): boolean {
  return freqHz > fLo && freqHz < fHi;
}

/**
 * `count` log-spaced frequencies over `[fLo, fHi]`, one per device-pixel column centre
 * (SPEC-015 §4.10: `f_lo * (f_hi/f_lo)^((i + 0.5) / count)`).
 */
export function logSpacedFreqs(fLo: number, fHi: number, count: number): number[] {
  if (count <= 0 || !(fHi > fLo)) {
    return [];
  }
  const ratio = Math.log(fHi / fLo);
  const out: number[] = new Array(count);
  for (let i = 0; i < count; i++) {
    const t = (i + 0.5) / count;
    out[i] = fLo * Math.exp(ratio * t);
  }
  return out;
}

export interface EqFreqTick {
  freqHz: number;
  /** Pixel x (0…width). */
  x: number;
  label: string;
}

const LADDER_MANTISSAS = [1, 2, 5] as const;

/** Every `{1, 2, 5} × 10ⁿ` Hz candidate inside `[fLo, fHi]` (same ladder as
 * `spectrum/freqAxis.ts::frequencyTicks`, duplicated here since that helper walks a *vertical*
 * axis and this one a horizontal one — only the label *formatting* is shared, via
 * `formatRulerFreqHz`, per the H-24 ticket's "reuse one shared frequency-tick generator ... where
 * possible"). */
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

/**
 * Horizontal frequency-axis ticks for the EQ graph (H-24 item 8): standard decade labels
 * ("20 50 100 200 500 1k 2k 5k 10k 20k"), thinned so labels never overlap at graph width
 * `widthPx`. Formatted with `spectrum/freqAxis.ts::formatRulerFreqHz` (the ticket's shared
 * generator, reused here for label text even though the log-frequency pixel math stays local to
 * this file per AC-17's import lint).
 */
export function eqFrequencyTicks(
  fLo: number,
  fHi: number,
  widthPx: number,
  minLabelGapPx: number,
  formatLabel: (freqHz: number) => string,
): EqFreqTick[] {
  if (!(fHi > fLo) || widthPx <= 0) {
    return [];
  }
  const candidates = ladderCandidates(fLo, fHi).sort((a, b) => a - b);
  const ticks: EqFreqTick[] = [];
  let lastX: number | null = null;
  for (const freqHz of candidates) {
    const x = xForFreq(freqHz, widthPx, fLo, fHi);
    if (lastX === null || Math.abs(x - lastX) >= minLabelGapPx) {
      ticks.push({ freqHz, x, label: formatLabel(freqHz) });
      lastX = x;
    }
  }
  return ticks;
}

/**
 * The frequencies to request from `rack_response_curve` (S3-07, SPEC-015 §4.10 lean form):
 * `columns` log-spaced points plus every band's exact frequency (so a narrow, high-Q apex is
 * never missed), sorted ascending, deduplicated, and capped at `maxPoints`.
 */
export function curveRequestFreqs(
  fLo: number,
  fHi: number,
  columns: number,
  bandFreqsHz: readonly number[],
  maxPoints: number,
): number[] {
  const cols = Math.max(0, Math.min(columns, maxPoints));
  const points = new Set<number>(logSpacedFreqs(fLo, fHi, cols));
  for (const f of bandFreqsHz) {
    if (f >= fLo && f <= fHi) {
      points.add(f);
    }
  }
  return [...points].sort((a, b) => a - b).slice(0, maxPoints);
}

/**
 * Harmonic analysis of a frozen voice spectrum (H-91 §2–§4): which of H1…H6 the measured
 * spectrum actually supports, whether the strongest peak in the spectrum *is* the fundamental
 * or a harmonic of it, and whether the spectrum agrees with the tracker about which octave the
 * fundamental is in.
 *
 * ## Harmonics live in a band, not at a frequency
 * The tracker reports F0 as a **time** median over the take, with a 10th–90th percentile range
 * (SPEC-007 §8.6); the spectrum is an **energy** average over that same take. So harmonic `n`
 * is not a line at `n · median` — its energy is spread over `n · low … n · high`, and the
 * spread grows with `n`. Every measurement here is therefore made over that band, and the two
 * consequences are taken seriously:
 *
 * - a harmonic's **peak** may legitimately sit tens of cents away from `n · median` (in the
 *   owner's own recording H2 peaks at 199 Hz while `2 × median` is 207 Hz — both are true, they
 *   are different averages);
 * - once `(n + 1) · low ≤ n · high` the bands of neighbouring harmonics **touch**, and no
 *   measurement can separate them. Those harmonics are reported `unresolved` rather than
 *   measured badly. A voice with a ±3-semitone range resolves H1 and H2 and nothing above.
 *
 * ## Support
 * A harmonic counts as supported when the maximum inside its band stands
 * {@link HARMONIC_MIN_PROMINENCE_DB} above the higher of the two valleys beside it (the gaps
 * `(n−1)·high … n·low` and `n·high … (n+1)·low`) — the same topographic prominence the peak
 * labels use, measured over the harmonic comb's own spacing instead of a fixed octave width.
 * H1 has no left neighbour, so only the valley above it counts.
 *
 * ## Octave
 * Two different octave errors need two different guards, and this module only owns the second:
 * - a *few frames* slipping is handled where the frames are (`vox_dsp::diagnostics::f0_profile`
 *   folds them back before any percentile is taken);
 * - the tracker locking onto **H2 for the whole take** is invisible there, because every frame
 *   agrees. Only the spectrum can catch it: if the fundamental really were an octave lower,
 *   there would be lines at 1.5·F0, 2.5·F0, 3.5·F0 — the harmonics of F0/2 that F0's own series
 *   does not contain. {@link chooseFundamental} looks for exactly those, and moves the
 *   fundamental down only when it finds them. It never moves it *up*: "F0 is weak" is normal for
 *   a voice and is never evidence that F0 isn't there.
 */
import { firstIndexAtOrAbove, refinePeakAt, type SpectrumCurveLike } from "../peaks";
import {
  HARMONIC_COUNT,
  HARMONIC_MIN_PROMINENCE_DB,
  HARMONIC_RANGE_MARGIN_CENTS,
  MAX_PEAK_HARMONIC,
  MIN_PITCH_HALF_RANGE_CENTS,
  MIN_SUPPORTED_SUB_HARMONICS,
  SUB_HARMONIC_TESTS,
} from "./thresholds";

/** The measured pitch range a harmonic band is built from (Hz). */
export interface PitchRange {
  medianHz: number;
  lowHz: number;
  highHz: number;
}

export type HarmonicStatus =
  /** A peak inside the band stands clear of the valleys beside it. */
  | "supported"
  /** The band was measurable, but nothing in it stands out. */
  | "weak"
  /** Neighbouring harmonics' bands touch at this `n`: nothing can be measured here. */
  | "unresolved";

export interface HarmonicMeasurement {
  /** 1 = the fundamental. Fractional for the sub-harmonic probes (1.5, 2.5, …). */
  n: number;
  /** `n · median` — where a *steady* voice would put this harmonic. */
  nominalHz: number;
  /** The band the harmonic's energy is spread over, `n · low … n · high` plus the margin. */
  bandHz: [number, number];
  /** The loudest point inside the band, parabolically refined; `null` when the band is off the
   * end of the curve. */
  peakHz: number | null;
  /** Its level (dB); `-Infinity` when there is nothing there. */
  levelDb: number;
  /** How far it stands above the valleys beside the band (dB); `null` when they are off-curve. */
  prominenceDb: number | null;
  status: HarmonicStatus;
}

const CENTS_PER_OCTAVE = 1200;

/** Interval from `b` to `a` in cents; `NaN` for non-positive inputs. */
export function cents(a: number, b: number): number {
  return a > 0 && b > 0 ? CENTS_PER_OCTAVE * Math.log2(a / b) : NaN;
}

/** The pitch range widened by {@link MIN_PITCH_HALF_RANGE_CENTS} where it is narrower than
 * that, so a steady or synthetic report still has a band to search. */
export function usableRange(range: PitchRange): PitchRange {
  const { medianHz } = range;
  if (!(medianHz > 0)) {
    return range;
  }
  const floor = 2 ** (-MIN_PITCH_HALF_RANGE_CENTS / CENTS_PER_OCTAVE);
  const ceil = 2 ** (MIN_PITCH_HALF_RANGE_CENTS / CENTS_PER_OCTAVE);
  return {
    medianHz,
    lowHz: Math.min(range.lowHz > 0 ? range.lowHz : medianHz, medianHz * floor),
    highHz: Math.max(range.highHz > 0 ? range.highHz : medianHz, medianHz * ceil),
  };
}

/**
 * The highest harmonic number still separable for `range`: harmonic `n` and `n + 1` occupy
 * `n·low … n·high` and `(n+1)·low … (n+1)·high`, which stay apart only while
 * `(n + 1)·low > n·high` — i.e. while `n < low / (high − low)`. The wider the speaker's pitch
 * range, the sooner the comb smears into a continuum; above this `n` nothing about an
 * individual harmonic is measurable, whatever the FFT resolution.
 */
export function highestSeparableHarmonic(range: PitchRange, max = MAX_PEAK_HARMONIC): number {
  const { lowHz, highHz } = usableRange(range);
  if (!(lowHz > 0) || !(highHz >= lowHz)) {
    return 0;
  }
  const ratio = highHz / lowHz;
  const n = ratio > 1 ? Math.floor(1 / (ratio - 1)) : max;
  return Math.max(0, Math.min(max, n));
}

/** `true` while harmonic `n`'s band stays clear of harmonic `n ± 1`'s. */
export function harmonicIsResolvable(n: number, range: PitchRange): boolean {
  return n >= 1 && n <= highestSeparableHarmonic(range, Number.MAX_SAFE_INTEGER);
}

/** The highest of H1…H{@link HARMONIC_COUNT} still separable for `range`. */
export function lastResolvableHarmonic(range: PitchRange): number {
  return highestSeparableHarmonic(range, HARMONIC_COUNT);
}

function maxInRange(
  curve: SpectrumCurveLike,
  lowHz: number,
  highHz: number,
): { index: number; levelDb: number } | null {
  const { freqsHz: freqs, levelsDb: levels } = curve;
  const n = Math.min(freqs.length, levels.length);
  const from = firstIndexAtOrAbove(freqs, lowHz, 0, n);
  const to = firstIndexAtOrAbove(freqs, highHz, from, n);
  let best = -1;
  let level = -Infinity;
  for (let i = from; i < Math.max(to, from + 1) && i < n; i++) {
    const v = levels[i] ?? -Infinity;
    if (v > level) {
      level = v;
      best = i;
    }
  }
  return best < 0 || !Number.isFinite(level) ? null : { index: best, levelDb: level };
}

function minInRange(curve: SpectrumCurveLike, lowHz: number, highHz: number): number | null {
  const { freqsHz: freqs, levelsDb: levels } = curve;
  const n = Math.min(freqs.length, levels.length);
  const from = firstIndexAtOrAbove(freqs, lowHz, 0, n);
  const to = firstIndexAtOrAbove(freqs, highHz, from, n);
  let level = Infinity;
  for (let i = from; i < to; i++) {
    const v = levels[i] ?? -Infinity;
    if (v < level) {
      level = v;
    }
  }
  return Number.isFinite(level) ? level : null;
}

const margin = 2 ** (HARMONIC_RANGE_MARGIN_CENTS / CENTS_PER_OCTAVE);

/** Harmonic `n`'s band for `range`, `[low, high]` Hz, widened by the range margin. */
export function harmonicBand(n: number, range: PitchRange): [number, number] {
  const { lowHz, highHz } = usableRange(range);
  return [(n * lowHz) / margin, n * highHz * margin];
}

/**
 * Measures harmonic `n` of `range` on `curve`. `n` may be fractional — the sub-harmonic probes
 * ({@link chooseFundamental}) measure 1.5, 2.5 and 3.5 the same way.
 */
export function measureHarmonic(
  curve: SpectrumCurveLike,
  n: number,
  range: PitchRange,
): HarmonicMeasurement {
  const usable = usableRange(range);
  const [bandLow, bandHigh] = harmonicBand(n, range);
  const nominalHz = n * usable.medianHz;
  const base: HarmonicMeasurement = {
    n,
    nominalHz,
    bandHz: [bandLow, bandHigh],
    peakHz: null,
    levelDb: -Infinity,
    prominenceDb: null,
    status: "weak",
  };
  // Integer harmonics stop being separable once the bands touch; the fractional probes are
  // measured against the *half*-spaced comb they belong to, so they use their own spacing.
  const spacing = Number.isInteger(n) ? 1 : 0.5;
  const resolvable = (n + spacing) * usable.lowHz > n * usable.highHz;
  if (!resolvable) {
    return { ...base, status: "unresolved" };
  }
  const peak = maxInRange(curve, bandLow, bandHigh);
  if (!peak) {
    return base;
  }
  const refined = refinePeakAt(curve, peak.index);
  // The valleys are the gaps between this harmonic's band and its neighbours' — measured on the
  // bands themselves, without the search margin, so the windows can't invert.
  const below = minInRange(curve, (n - spacing) * usable.highHz, n * usable.lowHz);
  const above = minInRange(curve, n * usable.highHz, (n + spacing) * usable.lowHz);
  // H1 (and the lowest probe) has no harmonic below it: only the valley above it can say
  // whether it stands out.
  const valley = n <= spacing ? above : Math.max(below ?? -Infinity, above ?? -Infinity);
  const prominenceDb = valley !== null && Number.isFinite(valley) ? refined.levelDb - valley : null;
  return {
    n,
    nominalHz,
    bandHz: [bandLow, bandHigh],
    peakHz: refined.freqHz,
    levelDb: refined.levelDb,
    prominenceDb,
    status: prominenceDb !== null && prominenceDb >= HARMONIC_MIN_PROMINENCE_DB ? "supported" : "weak",
  };
}

/** H1…H{@link HARMONIC_COUNT} of `range`, measured on `curve`. */
export function measureHarmonics(
  curve: SpectrumCurveLike,
  range: PitchRange,
  count = HARMONIC_COUNT,
): HarmonicMeasurement[] {
  const out: HarmonicMeasurement[] = [];
  for (let n = 1; n <= count; n++) {
    out.push(measureHarmonic(curve, n, range));
  }
  return out;
}

export interface FundamentalChoice {
  /** What the tracker reported (Hz). */
  trackedHz: number;
  /** What the report shows (Hz) — the tracker's value, or an octave below it. */
  fundamentalHz: number;
  /** `1` = the tracker's octave stands; `0.5` = the spectrum says it locked onto H2. */
  ratio: 1 | 0.5;
  /** `false` when the pitch range is too wide for the sub-harmonic lines to be separable — the
   * spectrum then has no opinion and the tracker's value stands unchallenged. */
  checked: boolean;
  /** The 1.5·F0 / 2.5·F0 / 3.5·F0 probes the decision was made from. */
  subHarmonics: HarmonicMeasurement[];
}

/**
 * Decides which octave the fundamental is in (see the module note). Only ever moves it **down**,
 * and only on positive evidence: {@link MIN_SUPPORTED_SUB_HARMONICS} of the lines that exist
 * solely under the octave-down reading must be there.
 */
export function chooseFundamental(curve: SpectrumCurveLike, range: PitchRange): FundamentalChoice {
  const trackedHz = range.medianHz;
  const subHarmonics = SUB_HARMONIC_TESTS.map((n) => measureHarmonic(curve, n, range));
  const measurable = subHarmonics.filter((h) => h.status !== "unresolved");
  const supported = subHarmonics.filter((h) => h.status === "supported");
  const checked = measurable.length >= MIN_SUPPORTED_SUB_HARMONICS;
  const halve = checked && supported.length >= MIN_SUPPORTED_SUB_HARMONICS;
  return {
    trackedHz,
    fundamentalHz: halve ? trackedHz / 2 : trackedHz,
    ratio: halve ? 0.5 : 1,
    checked,
    subHarmonics,
  };
}

export interface PeakRelation {
  /** The loudest peak in the spectrum (Hz) and its level (dB). */
  freqHz: number;
  levelDb: number;
  prominenceDb: number;
  /** `n` where the peak is harmonic `n` of the fundamental, else `null` (a formant region or a
   * room resonance that isn't on the comb). */
  harmonicNumber: number | null;
  /** The F0 this peak implies (`freqHz / n`), for the reader to compare with the measured
   * range; `null` when the peak is not a harmonic. */
  impliedF0Hz: number | null;
  /** How far the implied F0 sits from the tracked median (cents); `null` as above. */
  deviationCents: number | null;
  /** `true` only when the strongest peak in the whole spectrum is the fundamental itself. */
  isFundamental: boolean;
  /** Peak level − H1's measured level (dB); positive means the fundamental is not the loudest
   * thing in the voice. `null` when H1 could not be measured. */
  aboveFundamentalDb: number | null;
}

/**
 * Which harmonic of `range` a frequency is, or `null`. The test is the measured pitch range,
 * not a fixed tolerance: `freqHz / n` has to be a pitch this speaker actually used.
 */
export function harmonicNumberOf(freqHz: number, range: PitchRange): number | null {
  const usable = usableRange(range);
  if (!(freqHz > 0) || !(usable.medianHz > 0)) {
    return null;
  }
  const low = usable.lowHz / margin;
  const high = usable.highHz * margin;
  // Beyond the separable limit "the strongest peak is Hn" is not a claim the data supports —
  // Hn and Hn+1 overlap there — so the peak is reported as a region, not as a harmonic.
  const limit = highestSeparableHarmonic(range);
  const guess = Math.round(freqHz / usable.medianHz);
  let best: number | null = null;
  let bestOff = Infinity;
  for (const n of [guess - 1, guess, guess + 1]) {
    if (n < 1 || n > limit) {
      continue;
    }
    const implied = freqHz / n;
    if (implied < low || implied > high) {
      continue;
    }
    const off = Math.abs(cents(implied, usable.medianHz));
    if (off < bestOff) {
      bestOff = off;
      best = n;
    }
  }
  return best;
}

/**
 * Relates the loudest peak of `curve` to the fundamental (H-91 §3). `peakFinder` is the already
 * picked peak list (loudest first) — the snapshot picks them once and shares them.
 */
export function relateStrongestPeak(
  curve: SpectrumCurveLike,
  range: PitchRange,
  strongest: { freqHz: number; levelDb: number; prominenceDb: number } | undefined,
  harmonics: HarmonicMeasurement[],
): PeakRelation | null {
  if (!strongest) {
    return null;
  }
  const n = harmonicNumberOf(strongest.freqHz, range);
  const h1 = harmonics.find((h) => h.n === 1);
  const h1Level = h1 && Number.isFinite(h1.levelDb) && h1.status !== "unresolved" ? h1.levelDb : null;
  return {
    freqHz: strongest.freqHz,
    levelDb: strongest.levelDb,
    prominenceDb: strongest.prominenceDb,
    harmonicNumber: n,
    impliedF0Hz: n === null ? null : strongest.freqHz / n,
    deviationCents: n === null ? null : cents(strongest.freqHz / n, usableRange(range).medianHz),
    isFundamental: n === 1,
    aboveFundamentalDb: h1Level === null ? null : strongest.levelDb - h1Level,
  };
}

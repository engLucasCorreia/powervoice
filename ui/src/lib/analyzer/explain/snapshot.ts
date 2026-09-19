/**
 * The frozen analysis behind *Explain My Voice* (H-91 §1).
 *
 * One click freezes **one** immutable object: the spectrum as it was (its own copy of the
 * frequencies, the raw levels and the fractional-octave-smoothed envelope), the
 * `VoiceReportDto` the engine measured, and everything derived from the two — the pitch
 * profile, the harmonic measurements, the strongest peak's relation to the fundamental, and the
 * findings. It is built once, by {@link buildVoiceSnapshot}, and never recomputed: the live
 * analyzer keeps running underneath at frame rate, and nothing it does touches a frozen
 * snapshot.
 *
 * Everything in here is measured. The only constants involved are the documented thresholds in
 * `thresholds.ts`.
 */
import type { VoiceReportDto } from "../../ipc/bindings";
import { noteForFreq, type NoteInfo } from "../notes";
import { DEFAULT_PEAK_COUNT, findPeaks, type SpectralPeak } from "../peaks";
import { smoothFractionalOctave } from "../smoothing";
import { buildFindings, type VoiceFinding } from "./findings";
import {
  chooseFundamental,
  measureHarmonics,
  relateStrongestPeak,
  type FundamentalChoice,
  type HarmonicMeasurement,
  type PeakRelation,
  type PitchRange,
} from "./harmonics";
import { HARMONIC_COUNT } from "./thresholds";

/** H-92 draws the envelope at this width by default (octaves). */
export const DEFAULT_SNAPSHOT_SMOOTHING_OCT = 1 / 12;
/** How many peaks the snapshot picks for the plot; the loudest is the one §3 reasons about. */
export const SNAPSHOT_PEAK_COUNT = 8;
/** Peaks below this are not worth relating to anything (Hz, SPEC-007 §8.2's floor). */
export const PEAK_MIN_HZ = 20;

/** Where the frozen curve came from — the analyzer's own snapshot origins. */
export type VoiceSnapshotOrigin = "live" | "average" | "inspector" | "source" | "processed";

export interface VoiceSnapshotInput {
  /** Ascending frequencies (Hz). Copied. */
  freqsHz: ArrayLike<number>;
  /** Their levels (dB, `-Infinity` allowed). Copied. */
  levelsDb: ArrayLike<number>;
  /** `bands` = the live 1/24-octave curve, `bins` = FFT bins (`PlotCurve.resolution`). */
  resolution: "bands" | "bins";
  report: VoiceReportDto;
  sampleRateHz: number;
  origin: VoiceSnapshotOrigin;
  /** Envelope width, octaves (default {@link DEFAULT_SNAPSHOT_SMOOTHING_OCT}). */
  smoothingOct?: number;
  /** Injectable clock, for tests. */
  nowMs?: number;
}

/** The pitch profile the report shows: what the tracker measured, and which octave the
 * spectrum agrees the fundamental is in. */
export interface PitchProfile {
  /** The fundamental the report shows (Hz) — {@link FundamentalChoice.fundamentalHz}. */
  fundamentalHz: number;
  /** Its nearest note, for the readouts (`notes.ts`, A4 = 440). */
  note: NoteInfo | null;
  /** The tracker's median (Hz) and its 10th/90th percentiles. */
  medianHz: number;
  lowHz: number;
  highHz: number;
  /** Width of the 10th–90th range, in cents. */
  rangeCents: number;
  /** 1 − median aperiodicity over the voiced frames. */
  confidence: number;
  /** Share of voiced frames the engine folded back from an octave error. */
  octaveCorrected: number;
  voicedFraction: number;
  choice: FundamentalChoice;
}

export interface VoiceSnapshot {
  readonly takenAtMs: number;
  readonly origin: VoiceSnapshotOrigin;
  readonly sampleRateHz: number;
  readonly resolution: "bands" | "bins";
  readonly smoothingOct: number;
  /** The frozen curve: frequencies, the raw levels, and the smoothed envelope. */
  readonly freqsHz: Float64Array;
  readonly rawDb: Float32Array;
  readonly smoothedDb: Float32Array;
  readonly report: VoiceReportDto;
  /** Seconds of non-silent audio the report covers (`VoiceReport::span_s`). */
  readonly spanS: number;
  /** `null` when nothing voiced was measured. */
  readonly pitch: PitchProfile | null;
  /** H1…H6; `unresolved` where the pitch range makes them inseparable. */
  readonly harmonics: HarmonicMeasurement[];
  /** The loudest peaks of the frozen curve, loudest first (H-92 draws them). */
  readonly peaks: SpectralPeak[];
  /** How the loudest of them relates to the fundamental. */
  readonly strongestPeak: PeakRelation | null;
  readonly findings: VoiceFinding[];
}

function pitchRangeOf(report: VoiceReportDto): PitchRange | null {
  const f0 = report.f0;
  if (!f0 || !(f0.median_hz > 0)) {
    return null;
  }
  return { medianHz: f0.median_hz, lowHz: f0.low_hz, highHz: f0.high_hz };
}

/** Builds the frozen analysis. Pure: the same input always gives the same result (given
 * `nowMs`), and nothing it returns aliases the caller's arrays. */
export function buildVoiceSnapshot(input: VoiceSnapshotInput): VoiceSnapshot {
  const n = Math.min(input.freqsHz.length, input.levelsDb.length);
  const freqsHz = new Float64Array(n);
  const rawDb = new Float32Array(n);
  for (let i = 0; i < n; i++) {
    freqsHz[i] = input.freqsHz[i] ?? 0;
    rawDb[i] = input.levelsDb[i] ?? -Infinity;
  }
  const smoothingOct = input.smoothingOct ?? DEFAULT_SNAPSHOT_SMOOTHING_OCT;
  const smoothedDb = smoothFractionalOctave(freqsHz, rawDb, smoothingOct);
  const curve = { freqsHz, levelsDb: rawDb };

  const range = pitchRangeOf(input.report);
  const choice = range ? chooseFundamental(curve, range) : null;
  // Once the spectrum has had its say about the octave, every harmonic band is built from the
  // fundamental the report actually shows — not from the tracker's raw median.
  const reportedRange: PitchRange | null =
    range && choice
      ? {
          medianHz: choice.fundamentalHz,
          lowHz: range.lowHz * choice.ratio,
          highHz: range.highHz * choice.ratio,
        }
      : null;
  const harmonics = reportedRange ? measureHarmonics(curve, reportedRange, HARMONIC_COUNT) : [];

  const nyquistHz = input.sampleRateHz > 0 ? input.sampleRateHz / 2 : Infinity;
  const peaks = findPeaks(curve, {
    count: Math.max(SNAPSHOT_PEAK_COUNT, DEFAULT_PEAK_COUNT),
    fMinHz: PEAK_MIN_HZ,
    fMaxHz: nyquistHz,
  });
  const strongestPeak = reportedRange
    ? relateStrongestPeak(curve, reportedRange, peaks[0], harmonics)
    : null;

  const f0 = input.report.f0;
  const pitch: PitchProfile | null =
    reportedRange && choice && f0
      ? {
          fundamentalHz: choice.fundamentalHz,
          note: noteForFreq(choice.fundamentalHz),
          medianHz: reportedRange.medianHz,
          lowHz: reportedRange.lowHz,
          highHz: reportedRange.highHz,
          rangeCents: 1200 * Math.log2(Math.max(reportedRange.highHz, 1e-9) / Math.max(reportedRange.lowHz, 1e-9)),
          confidence: f0.confidence,
          octaveCorrected: f0.octave_corrected,
          voicedFraction: f0.voiced_fraction,
          choice,
        }
      : null;

  const findings = buildFindings({
    report: input.report,
    fundamental: choice,
    harmonics,
    strongestPeak,
  });

  return {
    takenAtMs: input.nowMs ?? Date.now(),
    origin: input.origin,
    sampleRateHz: input.sampleRateHz,
    resolution: input.resolution,
    smoothingOct,
    freqsHz,
    rawDb,
    smoothedDb,
    report: input.report,
    spanS: input.report.span_s,
    pitch,
    harmonics,
    peaks,
    strongestPeak,
    findings,
  };
}

/** The measured pitch range as the harmonic maths sees it (widened where it is degenerate) —
 * re-exported so the drawing layer anchors its bands to exactly the same numbers. */
export { usableRange } from "./harmonics";

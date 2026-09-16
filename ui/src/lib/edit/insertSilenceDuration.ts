import { parsePlainNumber, parseTimecodeValue } from "../waveform/timeFormat";

/**
 * H-56 (SPEC-008 §2.5): parses the Insert Silence dialog's Duration field. Unlike
 * `waveform/timeFormat.ts::parseDocumentTime` (one field, one fixed format bound to the
 * time-ruler setting), this one field accepts any of three notations at once, auto-detected from
 * the text itself:
 * - plain seconds: `1`, `1.5`, `0.250`;
 * - timecode `[[hh:]mm:]ss[.fff]`: `0:01.500`, `00:00:02`;
 * - an integer followed by `smp`: `48000 smp` (used as-is, no rounding).
 *
 * Seconds/timecode are converted with `round(d_s * sampleRateHz)` (f64, round half away from
 * zero — `Math.round` on a non-negative value is exactly that). Returns `null` for anything that
 * doesn't parse, or an out-of-range one — see {@link INSERT_SILENCE_MIN_SAMPLES} /
 * {@link insertSilenceMaxSamples}, which the dialog also uses to disable OK without recomputing
 * the bound.
 */
export function parseInsertSilenceDuration(text: string, sampleRateHz: number): number | null {
  const trimmed = text.trim();
  if (trimmed === "") {
    return null;
  }
  const smpMatch = /^(\d+)\s*smp$/i.exec(trimmed);
  if (smpMatch) {
    const n = Number(smpMatch[1]);
    return Number.isFinite(n) ? Math.round(n) : null;
  }
  if (!(sampleRateHz > 0)) {
    return null;
  }
  if (trimmed.includes(":")) {
    return parseTimecodeValue(trimmed, sampleRateHz);
  }
  const seconds = parsePlainNumber(trimmed);
  return seconds === null ? null : Math.round(seconds * sampleRateHz);
}

/** SPEC-008 §2.5: "From 1 sample to 3 600 s". */
export const INSERT_SILENCE_MIN_SAMPLES = 1;

/** The 1 h ceiling, in samples, at `sampleRateHz`. */
export function insertSilenceMaxSamples(sampleRateHz: number): number {
  return 3_600 * sampleRateHz;
}

/** `true` when `samples` (already parsed) is in the dialog's accepted range. */
export function insertSilenceInRange(samples: number, sampleRateHz: number): boolean {
  return (
    Number.isFinite(samples) &&
    samples >= INSERT_SILENCE_MIN_SAMPLES &&
    samples <= insertSilenceMaxSamples(sampleRateHz)
  );
}

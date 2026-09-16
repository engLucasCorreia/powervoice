/**
 * The one document-time formatter/parser (T-206, SPEC-006 §2.5: "every format the spec lists ...
 * switchable from the time display and View" — `timecode`, `samples`, `seconds`). Every UI
 * surface that shows or edits a document sample position as text goes through this module: the
 * toolbar clock, the time ruler's tick labels (`timeRuler.ts` builds on the same primitives), the
 * marker list, and the selection start/end/length readouts.
 *
 * `timecode` reuses `transport/playhead.ts::formatTime` verbatim (always `hh:mm:ss.mmm`,
 * millisecond precision — unchanged from before this ticket, so every existing caller of
 * `formatTime` keeps its exact output). `samples` and `seconds` are new. Parsing is the inverse
 * for editable fields (selection readouts): `samples` and `seconds` round-trip to the exact
 * sample (seconds at 6 decimals, enough resolution for any realistic sample rate); `timecode`
 * round-trips only to millisecond precision, same limit as its display.
 */

import type { TimeRulerFormatDto } from "../ipc/bindings";
import { formatTime } from "../transport/playhead";

export type TimeRulerFormat = TimeRulerFormatDto;

/** Decimal places `seconds` format shows/parses — enough to be sample-exact at any realistic
 * sample rate (a µs-level 6th decimal is ≪ 1 sample even at 192 kHz). */
export const SECONDS_EDIT_DECIMALS = 6;

export function formatSamplesValue(samples: number): string {
  return String(Math.max(0, Math.round(samples)));
}

export function formatSecondsValue(
  samples: number,
  sampleRateHz: number,
  decimals: number = SECONDS_EDIT_DECIMALS,
): string {
  if (!(sampleRateHz > 0)) {
    return (0).toFixed(decimals);
  }
  return (Math.max(0, samples) / sampleRateHz).toFixed(decimals);
}

/** Dispatches to the current `time_ruler_format` (SPEC-006 §2.2's `timeRulerFormat`). */
export function formatDocumentTime(
  samples: number,
  sampleRateHz: number,
  format: TimeRulerFormat,
): string {
  switch (format) {
    case "samples":
      return formatSamplesValue(samples);
    case "seconds":
      return formatSecondsValue(samples, sampleRateHz);
    case "timecode":
    default:
      return formatTime(samples, sampleRateHz);
  }
}

/** Parses `[hh:]mm:ss[.fff]` (a bare `ss[.fff]` group is accepted too) into samples, or `null` for
 * anything else. Matches exactly what {@link formatTime} writes, plus the shorter forms a user
 * might type. */
export function parseTimecodeValue(text: string, sampleRateHz: number): number | null {
  if (!(sampleRateHz > 0)) {
    return null;
  }
  const parts = text.split(":");
  if (parts.length < 1 || parts.length > 3) {
    return null;
  }
  const secMatch = /^(\d{1,2})(?:\.(\d{1,3}))?$/.exec(parts[parts.length - 1] ?? "");
  if (!secMatch) {
    return null;
  }
  const seconds = Number(secMatch[1]);
  if (seconds >= 60) {
    return null;
  }
  const fracStr = secMatch[2] ?? "";
  const frac = fracStr ? Number(fracStr.padEnd(3, "0")) / 1000 : 0;

  let minutes = 0;
  if (parts.length >= 2) {
    const m = /^(\d+)$/.exec(parts[parts.length - 2] ?? "");
    if (!m) {
      return null;
    }
    minutes = Number(m[1]);
    if (parts.length === 3 && minutes >= 60) {
      return null;
    }
  }
  let hours = 0;
  if (parts.length === 3) {
    const h = /^(\d+)$/.exec(parts[0] ?? "");
    if (!h) {
      return null;
    }
    hours = Number(h[1]);
  }
  const totalSeconds = hours * 3600 + minutes * 60 + seconds + frac;
  return Math.round(totalSeconds * sampleRateHz);
}

/** A plain (unsigned) number: digits, an optional single `.` or `,` decimal separator. Document
 * positions are never negative, so this intentionally doesn't accept `units.ts::parseNumber`'s
 * minus-sign glyphs. */
export function parsePlainNumber(text: string): number | null {
  const normalized = text.trim().replace(",", ".");
  if (!/^\d+\.?\d*$|^\.\d+$/.test(normalized)) {
    return null;
  }
  const value = Number(normalized);
  return Number.isFinite(value) ? value : null;
}

/** The inverse of {@link formatDocumentTime}, for editable fields (selection readouts). `null`
 * for anything that doesn't parse cleanly in `format`. */
export function parseDocumentTime(
  text: string,
  sampleRateHz: number,
  format: TimeRulerFormat,
): number | null {
  const trimmed = text.trim();
  if (trimmed === "") {
    return null;
  }
  switch (format) {
    case "samples": {
      const n = parsePlainNumber(trimmed);
      return n === null ? null : Math.round(n);
    }
    case "seconds": {
      if (!(sampleRateHz > 0)) {
        return null;
      }
      const n = parsePlainNumber(trimmed);
      return n === null ? null : Math.round(n * sampleRateHz);
    }
    case "timecode":
    default:
      return parseTimecodeValue(trimmed, sampleRateHz);
  }
}

/** A readout field is never sized smaller than this many characters, even for a very short
 * document — a one- or two-digit box reads as broken, not just "sized to fit" (H-48 item 1). */
const MIN_TIME_FIELD_CHARS = 4;

/**
 * How many characters (`ch`, tabular figures) a Start/End/Length readout field needs to show any
 * value this document/format pair can produce, without clipping (H-48 item 1 owner report: the
 * toolbar's selection fields clipped, e.g. "00:00:19.(" out of a fixed 8ch box). The worst case
 * for every one of the three fields is the document's own length in samples — `Start`/`End` can
 * reach it directly, and `Length` reaches it once the whole document is selected — so sizing every
 * field to that one value's formatted width keeps the three fields aligned as well.
 */
export function documentTimeFieldChars(
  sampleRateHz: number,
  lenSamples: number,
  format: TimeRulerFormat,
): number {
  const worstCase = formatDocumentTime(Math.max(0, lenSamples), sampleRateHz, format).length;
  return Math.max(worstCase, MIN_TIME_FIELD_CHARS);
}

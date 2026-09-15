/**
 * T-304 (SPEC-022): pure helpers for the record panel's Punch & pre-roll section — the phase
 * label with its countdown (§2.11) and the recording-offset entry/readout (§2.13). Canvas- and
 * store-free so they are unit-testable.
 */

import { formatNumber } from "../ui/units";
import type { RecordOffsetDto, RecordPhaseDto, RecordPrefsDto, RecordStartedDto } from "../ipc/bindings";

/** SPEC-022 §2.3 factory defaults (used until settings load). */
export const DEFAULT_RECORD_PREFS: RecordPrefsDto = {
  mode: "insert",
  punch_on_selection: true,
  preroll_s: 5,
  postroll_s: 1,
  preroll_at_cursor: false,
  hear_original: false,
  punch_xfade_ms: 10,
};

/** SPEC-022 §3 ranges. */
export const ROLL_MAX_S = 20;
export const XFADE_MAX_MS = 50;
export const OFFSET_MAX_MS = 500;

/** `m:ss.t` of `samples` at `rateHz` (the punch countdown, "0:01.3"). */
export function formatClock(samples: number, rateHz: number): string {
  if (rateHz <= 0 || !Number.isFinite(samples)) {
    return "0:00.0";
  }
  const tenths = Math.max(0, Math.floor((samples / rateHz) * 10));
  const m = Math.floor(tenths / 600);
  const s = Math.floor((tenths % 600) / 10);
  return `${m}:${String(s).padStart(2, "0")}.${tenths % 10}`;
}

/** The i18n key and params of the record panel's phase label (SPEC-022 §2.11), or `null`. */
export function phaseLabel(
  op: RecordStartedDto | null,
  phase: RecordPhaseDto | null,
  heardSamples: number,
  rateHz: number,
): { key: string; params: Record<string, string> } | null {
  if (!op || !phase) {
    return null;
  }
  switch (phase.phase) {
    case "preroll": {
      const left = Math.max(0, op.at_samples - heardSamples) / Math.max(1, rateHz);
      return { key: "record.phase.preroll", params: { time: left.toFixed(1) } };
    }
    case "recording": {
      const elapsed = formatClock(heardSamples - op.at_samples, rateHz);
      if (op.op === "punch" && op.end_samples !== null) {
        return {
          key: "record.phase.punch",
          params: { elapsed, total: formatClock(op.end_samples - op.at_samples, rateHz) },
        };
      }
      return { key: "record.phase.recording", params: { elapsed } };
    }
    case "postroll":
      return { key: "record.phase.postroll", params: {} };
    case "committing":
      return { key: "record.phase.committing", params: {} };
  }
}

/**
 * Parses a manual recording-offset entry (SPEC-022 §2.13): milliseconds ("3.2", "-1.5 ms") or
 * samples ("150 smp", "150 samples") converted at the device rate; clamped to ±500 ms. `null` for
 * anything else.
 */
export function parseOffsetText(text: string, deviceRateHz: number): number | null {
  const m = /^\s*([+-]?\d+(?:[.,]\d+)?)\s*(ms|smp|samples?)?\s*$/i.exec(text);
  if (!m || m[1] === undefined) {
    return null;
  }
  const value = Number(m[1].replace(",", "."));
  if (!Number.isFinite(value)) {
    return null;
  }
  const unit = (m[2] ?? "ms").toLowerCase();
  let ms = value;
  if (unit !== "ms") {
    if (deviceRateHz <= 0) {
      return null;
    }
    ms = (value / deviceRateHz) * 1000;
  }
  return Math.max(-OFFSET_MAX_MS, Math.min(OFFSET_MAX_MS, ms));
}

/** The offset readout's i18n key and params (SPEC-022 §2.13 "Offset +3.00 ms (144 smp) · …"). */
export function offsetReadout(
  offset: RecordOffsetDto | null,
): { key: string; params: Record<string, string> } {
  if (!offset || offset.source === null) {
    return { key: "record.offset.not_calibrated", params: {} };
  }
  const ms = formatNumber(offset.offset_ms, 2, { signed: true });
  const samples = formatNumber(Math.round((offset.offset_ms / 1000) * offset.device_rate_hz), 0);
  if (offset.source === "manual") {
    return { key: "record.offset.manual", params: { ms, samples } };
  }
  const date =
    offset.updated_unix_ms === null
      ? ""
      : new Date(offset.updated_unix_ms).toLocaleDateString(undefined, {
          day: "numeric",
          month: "short",
          year: "numeric",
        });
  return { key: "record.offset.calibrated", params: { ms, samples, date } };
}

/** The amber "recalibrate" hint when the buffer size changed since calibration (§2.13). */
export function bufferHint(offset: RecordOffsetDto | null): { was: string; now: string } | null {
  if (
    !offset ||
    offset.source !== "calibrated" ||
    offset.buffer_frames === null ||
    offset.current_buffer_frames === null ||
    offset.buffer_frames === offset.current_buffer_frames
  ) {
    return null;
  }
  return { was: String(offset.buffer_frames), now: String(offset.current_buffer_frames) };
}

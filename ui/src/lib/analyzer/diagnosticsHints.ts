/**
 * Plain-language readings of a voice report (H-42, SPEC-007 §8.10): each measurement becomes a
 * finding with a severity, an i18n hint and, where an EQ move helps, an "Add EQ band here"
 * action (a notch for hum, a peak cut/boost, a high-pass for rumble). The de-esser suggestion
 * only copies its frequency — there is no de-esser module yet.
 *
 * The thresholds are voice-over rules of thumb, not standards (documented in the SPEC-007
 * amendment). Tone balance is measured as per-octave density relative to the 1 kHz octave, where
 * pink noise reads 0 dB; a typical voice sits around +3 (mud), −7 (presence), −22 dB (air).
 *
 * These hints and the *Explain My Voice* prose (`explain/prose.ts`, H-94) describe the same
 * measurements and must never disagree in register (H-99, SPEC-007 Amendment 2): air is a
 * description, never a boost to reach for, and "boomy"/"harsh" wording (with the EQ move that
 * goes with it) is reserved for a reading that clears its warn threshold by more than
 * {@link NEAR_THRESHOLD_DB} — the same margin Explain uses, so the two never split a hair
 * differently. Panel copy stays a terse one-liner; only the verdict has to match, not the length.
 */
import type { MessageKey, MessageParams } from "../i18n";
import type { VoiceReportDto } from "../ipc/bindings";
import { formatNumber } from "../ui/units";

export type FindingId = "f0" | "mud" | "presence" | "air" | "sibilance" | "hum" | "rumble" | "noise" | "snr";
export type Severity = "ok" | "info" | "warn";
export type EqActionKind = "notch" | "cut" | "boost" | "high_pass";

export interface EqAction {
  kind: EqActionKind;
  freqHz: number;
  gainDb: number;
  q: number;
}

export type FindingAction = { type: "eq"; eq: EqAction } | { type: "copy"; freqHz: number };

export interface Finding {
  id: FindingId;
  severity: Severity;
  hintKey: MessageKey;
  params?: MessageParams;
  action?: FindingAction;
}

/** Tone-balance zones (dB relative to the 1 kHz octave): below `low` / above `high` is a
 * finding; `warnHigh` escalates it to "boomy" + an EQ move, past {@link NEAR_THRESHOLD_DB} of
 * margin (H-99). Presence has no separate info step — crossing `high`/`low` at all is the warn
 * threshold, same as SPEC-007 §8.10's table. */
export const TONE_ZONES = {
  mud: { low: -6, high: 6, warnHigh: 9 },
  presence: { low: -14, high: -2 },
  air: { low: -35, high: -12 },
} as const;
/** Sibilance (4–10 kHz vs overall, dB). */
export const SIBILANCE_MODERATE_DB = -22;
export const SIBILANCE_STRONG_DB = -12;
/** Rumble (20–80 Hz vs overall, dB) above which a high-pass is suggested. */
export const RUMBLE_WARN_DB = -25;
/** ACX noise-floor ceiling (dBFS RMS). */
export const ACX_NOISE_FLOOR_DBFS = -60;
/** SNR bands (dB). */
export const SNR_FAIR_DB = 30;
export const SNR_GOOD_DB = 40;

/**
 * How far past a warn threshold (mud, presence) a reading must land before the panel calls it
 * "boomy"/"harsh" and offers an EQ move (H-99, SPEC-007 Amendment 2). Within this margin the
 * reading gets the same mild, no-action wording as the softer info band below the threshold —
 * a microphone swap or a few centimetres of working distance moves a band by this much on its
 * own, so it isn't evidence of anything to correct. Shared value with Explain My Voice's
 * `NEAR_THRESHOLD_DB` (`explain/thresholds.ts`, H-94): the panel and the modal must never split
 * the same hair of margin into two different verdicts.
 */
export const NEAR_THRESHOLD_DB = 1;

/** The EQ moves the findings offer. There is deliberately no move for "more air": a voice's
 * high end is supposed to roll off, so nothing here ever suggests boosting it (H-99). */
export const EQ_MOVES = {
  mudCut: { kind: "cut", freqHz: 300, gainDb: -3, q: 1.4 },
  presenceBoost: { kind: "boost", freqHz: 3500, gainDb: 2.5, q: 1 },
  presenceCut: { kind: "cut", freqHz: 3500, gainDb: -3, q: 2 },
  rumbleHighPass: { kind: "high_pass", freqHz: 80, gainDb: 0, q: 0.7071 },
} as const satisfies Record<string, EqAction>;
/** Hum notch: deep and narrow (RBJ peaking, SPEC-015 limits: gain ≥ −24 dB, Q ≤ 30). */
export const HUM_NOTCH_GAIN_DB = -20;
export const HUM_NOTCH_Q = 20;

/** "50 Hz", "315 Hz", "6.3 kHz", "12 kHz". */
export function formatFreqShort(freqHz: number): string {
  if (freqHz < 1000) {
    return `${formatNumber(Math.round(freqHz), 0)} Hz`;
  }
  const k = freqHz / 1000;
  return `${formatNumber(k, k < 10 ? 1 : 0)} kHz`;
}

function tone(report: VoiceReportDto): Finding[] {
  const out: Finding[] = [];
  const t = report.tone;
  if (!t) {
    return out;
  }
  const mud = t.mud_db;
  if (mud > TONE_ZONES.mud.warnHigh + NEAR_THRESHOLD_DB) {
    out.push({ id: "mud", severity: "warn", hintKey: "analyzer.hint.mud_high", action: { type: "eq", eq: EQ_MOVES.mudCut } });
  } else if (mud > TONE_ZONES.mud.high) {
    // Below the warn line, and a hair past it, both read as a description, not a defect: no
    // "boomy" wording and nothing to fix (H-99 — matches Explain's info branch).
    out.push({ id: "mud", severity: "info", hintKey: "analyzer.hint.mud_slight" });
  } else if (mud < TONE_ZONES.mud.low) {
    out.push({ id: "mud", severity: "info", hintKey: "analyzer.hint.mud_low" });
  } else {
    out.push({ id: "mud", severity: "ok", hintKey: "analyzer.hint.mud_ok" });
  }
  if (t.presence_db !== null) {
    const p = t.presence_db;
    if (p < TONE_ZONES.presence.low) {
      out.push({ id: "presence", severity: "warn", hintKey: "analyzer.hint.presence_low", action: { type: "eq", eq: EQ_MOVES.presenceBoost } });
    } else if (p > TONE_ZONES.presence.high + NEAR_THRESHOLD_DB) {
      out.push({ id: "presence", severity: "warn", hintKey: "analyzer.hint.presence_high", action: { type: "eq", eq: EQ_MOVES.presenceCut } });
    } else if (p > TONE_ZONES.presence.high) {
      // Barely across the "forward/harsh" line: say "forward", not "harsh", and offer nothing to
      // correct (H-99 — the same margin Explain treats as barely across).
      out.push({ id: "presence", severity: "info", hintKey: "analyzer.hint.presence_high_near" });
    } else {
      out.push({ id: "presence", severity: "ok", hintKey: "analyzer.hint.presence_ok" });
    }
  }
  if (t.air_db !== null) {
    const a = t.air_db;
    if (a < TONE_ZONES.air.low) {
      // Description only — a voice's top end is supposed to roll off, so this never offers a
      // boost to "fix" it (H-99, SPEC-007 Amendment 2).
      out.push({ id: "air", severity: "info", hintKey: "analyzer.hint.air_low" });
    } else if (a > TONE_ZONES.air.high) {
      out.push({ id: "air", severity: "info", hintKey: "analyzer.hint.air_high" });
    } else {
      out.push({ id: "air", severity: "ok", hintKey: "analyzer.hint.air_ok" });
    }
  }
  return out;
}

/** Every finding for `report`, in panel order. */
export function assessReport(report: VoiceReportDto): Finding[] {
  const out: Finding[] = [];
  if (report.f0) {
    out.push({ id: "f0", severity: "ok", hintKey: "analyzer.hint.f0" });
  }
  out.push(...tone(report));

  const s = report.sibilance;
  if (s) {
    const severity: Severity =
      s.ratio_db > SIBILANCE_STRONG_DB ? "warn" : s.ratio_db > SIBILANCE_MODERATE_DB ? "info" : "ok";
    const hintKey: MessageKey =
      severity === "warn"
        ? "analyzer.hint.sibilance_strong"
        : severity === "info"
          ? "analyzer.hint.sibilance_moderate"
          : "analyzer.hint.sibilance_ok";
    out.push({
      id: "sibilance",
      severity,
      hintKey,
      params: { freq: formatFreqShort(s.centre_hz) },
      action: { type: "copy", freqHz: s.centre_hz },
    });
  }

  if (report.hum) {
    const h = report.hum;
    const extra = h.harmonics.some((k) => k > 1);
    out.push({
      id: "hum",
      severity: "warn",
      hintKey: extra ? "analyzer.hint.hum_harmonics" : "analyzer.hint.hum",
      params: { freq: formatFreqShort(h.mains_hz) },
      action: {
        type: "eq",
        eq: { kind: "notch", freqHz: h.strongest_hz, gainDb: HUM_NOTCH_GAIN_DB, q: HUM_NOTCH_Q },
      },
    });
  } else if (report.noise_floor_dbfs !== null) {
    out.push({ id: "hum", severity: "ok", hintKey: "analyzer.hint.no_hum" });
  }

  if (report.rumble_db !== null) {
    out.push(
      report.rumble_db > RUMBLE_WARN_DB
        ? { id: "rumble", severity: "warn", hintKey: "analyzer.hint.rumble", action: { type: "eq", eq: EQ_MOVES.rumbleHighPass } }
        : { id: "rumble", severity: "ok", hintKey: "analyzer.hint.rumble_ok" },
    );
  }

  if (report.noise_floor_dbfs !== null) {
    out.push(
      report.noise_floor_dbfs > ACX_NOISE_FLOOR_DBFS
        ? { id: "noise", severity: "warn", hintKey: "analyzer.hint.noise_acx_fail" }
        : { id: "noise", severity: "ok", hintKey: "analyzer.hint.noise_ok" },
    );
  }
  if (report.snr_db !== null) {
    const snr = report.snr_db;
    out.push({
      id: "snr",
      severity: snr < SNR_FAIR_DB ? "warn" : snr < SNR_GOOD_DB ? "info" : "ok",
      hintKey: snr < SNR_FAIR_DB ? "analyzer.hint.snr_low" : snr < SNR_GOOD_DB ? "analyzer.hint.snr_fair" : "analyzer.hint.snr_good",
    });
  }
  return out;
}

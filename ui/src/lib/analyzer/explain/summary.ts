/**
 * The engineering summary of *Explain My Voice* (H-94 §2–§3): a voice profile composed from the
 * real measurements, a short suggested focus composed from the findings that actually crossed a
 * threshold, and — when the measurements justify one — a conservative set of EQ moves to draw
 * over the spectrum as a suggestion.
 *
 * Three rules shape it:
 *
 * - **the profile is numbers.** Each line carries the measurement and a plain reading of where
 *   it sits; where a measurement was not taken the line says so rather than filling in.
 * - **the focus is short, and empty when it should be.** A clean recording is told it is clean.
 *   Nothing is invented per section so the panel looks busy, and a reading that only just
 *   crossed its threshold appears as something to listen to, not something to correct.
 * - **the source comes before the processor.** Microphone distance, axis and the room are
 *   suggested before any EQ, because they are where the measurement actually came from.
 *
 * The EQ suggestion is emitted as {@link EqAction}s — the same shape `analyzer/eqSuggest.ts`
 * plans and `analyzer/eqApply.ts` applies — so "apply this suggestion" is the existing "Add EQ
 * band here" path with no second mechanism, and the drawn overlay is H-92's to render from the
 * same list.
 */
import { t, type MessageKey } from "../../i18n";
import { formatNumber } from "../../ui/units";
import { formatFreqShort, type EqAction, type FindingAction } from "../diagnosticsHints";
import { formatCents, noteName } from "../notes";
import { findingsNeedingAttention, type FindingId, type FindingSeverity, type VoiceFinding } from "./findings";
import { explainFindings, floorLooksProcessed, isNearThreshold, marginDb, shortName } from "./prose";
import type { VoiceSnapshot } from "./snapshot";

/** The six readings the profile shows, in the order it shows them. */
export type ProfileId = "f0" | "body" | "presence" | "sibilance" | "rumble" | "hum";

export interface ProfileEntry {
  id: ProfileId;
  label: string;
  /** The measurement, formatted. "not measured" when it was not taken. */
  value: string;
  /** Where that measurement sits, in words. */
  reading: string;
  /** `null` when the measurement was not taken. */
  severity: FindingSeverity | null;
}

export interface FocusItem {
  /** The finding it came from, or `noise_processing` for the "this file looks processed" note. */
  id: FindingId | "noise_processing";
  text: string;
  /** The EQ move or de-esser frequency behind it, where there is one. */
  action: FindingAction | null;
}

export interface VoiceSummary {
  /** What crossed a threshold, or that nothing did. */
  headline: string;
  /** How much audio the measurements cover. */
  basis: string;
  profile: ProfileEntry[];
  focus: FocusItem[];
  /** Conservative EQ moves the measurements justify, highest-priority finding first. */
  eqBands: EqAction[];
}

const PROFILE_LABELS: Record<ProfileId, MessageKey> = {
  f0: "explain.summary.profile.pitch",
  body: "explain.summary.profile.body",
  presence: "explain.summary.profile.presence",
  sibilance: "explain.summary.profile.sibilance",
  rumble: "explain.summary.profile.rumble",
  hum: "explain.summary.profile.hum",
};

function dbRelative(value: number): string {
  return `${formatNumber(value, 1, { signed: true })} dB`;
}

function dbSpan(value: number): string {
  return `${formatNumber(Math.abs(value), 1)} dB`;
}

/** Where a measurement sits, from its classification and how far past the line it landed. */
function readingKey(finding: VoiceFinding | undefined): MessageKey {
  if (!finding) {
    return "explain.summary.reading.not_measured";
  }
  if (finding.id === "hum") {
    return finding.zone === "absent"
      ? "explain.summary.reading.absent"
      : "explain.summary.reading.present";
  }
  if (finding.severity === "good" || finding.zone === "in_range") {
    return "explain.summary.reading.in_range";
  }
  const above = finding.zone === "above";
  if (finding.severity === "significant") {
    return above ? "explain.summary.reading.well_above" : "explain.summary.reading.well_below";
  }
  if (isNearThreshold(finding)) {
    return above ? "explain.summary.reading.just_above" : "explain.summary.reading.just_below";
  }
  return above ? "explain.summary.reading.above" : "explain.summary.reading.below";
}

function entry(id: ProfileId, finding: VoiceFinding | undefined, value: string, reading?: MessageKey): ProfileEntry {
  return {
    id,
    label: t(PROFILE_LABELS[id]),
    value: finding ? value : t("explain.summary.reading.not_measured"),
    reading: t(reading ?? readingKey(finding)),
    severity: finding?.severity ?? null,
  };
}

function profileOf(snapshot: VoiceSnapshot): ProfileEntry[] {
  const by = (id: FindingId) => snapshot.findings.find((f) => f.id === id);
  const pitch = snapshot.pitch;
  const f0 = by("f0");
  const body = by("body");
  const presence = by("presence");
  const sibilance = by("sibilance");
  const rumble = by("rumble");
  const hum = by("hum");
  return [
    entry(
      "f0",
      f0,
      pitch
        ? t("explain.summary.profile.pitch_value", {
            median: formatFreqShort(pitch.fundamentalHz),
            note: pitch.note ? `${noteName(pitch.note)} ${formatCents(pitch.note.cents)}¢` : "",
            low: formatNumber(pitch.lowHz, 0),
            high: formatFreqShort(pitch.highHz),
          })
        : "",
      // The pitch line is a measurement and says so: H-91 attaches no voice-type label to it.
      f0 ? "explain.summary.reading.pitch" : undefined,
    ),
    entry("body", body, body ? dbRelative(body.measured.value) : ""),
    entry("presence", presence, presence ? dbRelative(presence.measured.value) : ""),
    entry(
      "sibilance",
      sibilance,
      sibilance
        ? t("explain.summary.profile.sibilance_value", {
            value: dbRelative(sibilance.measured.value),
            freq: formatFreqShort(sibilance.detail.centreHz ?? 0),
          })
        : "",
    ),
    entry("rumble", rumble, rumble ? dbRelative(rumble.measured.value) : ""),
    entry(
      "hum",
      hum,
      hum && hum.zone !== "absent"
        ? t("explain.summary.profile.hum_value", {
            prominence: dbSpan(hum.measured.value),
            freq: formatFreqShort(hum.detail.strongestHz ?? 0),
          })
        : t("explain.summary.profile.hum_none"),
    ),
  ];
}

/** The focus line for one finding that crossed a threshold, or `null` when it has none. */
function focusLine(finding: VoiceFinding): { key: MessageKey; params?: Record<string, string> } | null {
  const margin = dbSpan(marginDb(finding) ?? 0);
  const near = isNearThreshold(finding);
  switch (finding.id) {
    case "body":
      // Lean is never escalated (H-91: there is no "too thin to be allowed"), so only the
      // elevated side ever reaches the focus list.
      if (finding.zone !== "above") {
        return null;
      }
      return near
        ? { key: "explain.summary.focus.body_near", params: { margin } }
        : { key: "explain.summary.focus.body" };
    case "presence":
      if (finding.zone === "below") {
        return { key: "explain.summary.focus.presence_low" };
      }
      return near
        ? { key: "explain.summary.focus.presence_high_near", params: { margin } }
        : { key: "explain.summary.focus.presence_high" };
    case "sibilance":
      return {
        key: "explain.summary.focus.sibilance",
        params: { freq: formatFreqShort(finding.detail.centreHz ?? 0) },
      };
    case "hum":
      return {
        key: "explain.summary.focus.hum",
        params: { freq: formatFreqShort(finding.detail.strongestHz ?? 0) },
      };
    case "rumble":
      return { key: "explain.summary.focus.rumble" };
    case "noise_floor":
      return { key: "explain.summary.focus.noise_floor" };
    case "snr":
      return { key: "explain.summary.focus.snr" };
    default:
      // Pitch, the strongest partial, the harmonic series and air describe the voice. There is
      // nothing to act on, so they never appear here.
      return null;
  }
}

function focusOf(snapshot: VoiceSnapshot): FocusItem[] {
  const prose = explainFindings(snapshot);
  const actionable: FocusItem[] = [];
  for (const finding of findingsNeedingAttention(snapshot.findings)) {
    const line = focusLine(finding);
    if (!line) {
      continue;
    }
    actionable.push({
      id: finding.id,
      text: t(line.key, line.params),
      action: prose.find((p) => p.id === finding.id)?.action ?? null,
    });
  }
  // A clean recording is told it is clean, once, instead of being given a line per section.
  const out: FocusItem[] =
    actionable.length > 0
      ? actionable
      : [{ id: "noise_floor", text: t("explain.summary.focus.none"), action: null }];
  // An observation, not an action: nothing crossed, but the number would be read wrongly if the
  // reader were not told what it probably is.
  if (floorLooksProcessed(snapshot.report.noise_floor_dbfs)) {
    out.push({
      id: "noise_processing",
      text: t("explain.summary.focus.processed_floor"),
      action: null,
    });
  }
  return out;
}

function headlineOf(snapshot: VoiceSnapshot): string {
  const crossed = findingsNeedingAttention(snapshot.findings);
  if (crossed.length === 0) {
    return t("explain.summary.headline.clean");
  }
  const list = crossed.map((f) => shortName(f.id)).join(", ");
  return crossed.length === 1
    ? t("explain.summary.headline.one", { list })
    : t("explain.summary.headline.many", { count: crossed.length, list });
}

/**
 * The conservative EQ moves the measurements justify, in finding-priority order — the dashed
 * suggestion curve H-92 draws over (never into) the measured spectrum, and the list "apply"
 * hands to `eqApply.ts`. Empty whenever nothing justifies a move, including for a reading that
 * only just crossed its threshold.
 *
 * Tone moves are capped at `MAX_SUGGESTED_TONE_GAIN_DB` where they are defined
 * (`diagnosticsHints.EQ_MOVES`); a mains notch and a high-pass are corrective filters aimed at
 * something that is not the voice, and are not tone shaping.
 */
export function suggestedEqBands(snapshot: VoiceSnapshot): EqAction[] {
  return explainFindings(snapshot)
    .filter((p) => p.action?.type === "eq")
    .map((p) => (p.action as { type: "eq"; eq: EqAction }).eq);
}

/** The whole summary for a frozen snapshot. */
export function buildVoiceSummary(snapshot: VoiceSnapshot): VoiceSummary {
  return {
    headline: headlineOf(snapshot),
    basis: t("explain.summary.basis", { span: `${formatNumber(snapshot.spanS, 1)} s` }),
    profile: profileOf(snapshot),
    focus: focusOf(snapshot),
    eqBands: suggestedEqBands(snapshot),
  };
}

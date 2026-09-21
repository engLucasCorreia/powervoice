/**
 * The words of *Explain My Voice* (H-94): one title, one measured sentence, one interpretation
 * and — only where the measurement justifies it — one recommendation, per finding from H-91.
 *
 * ## The rule this module exists to enforce
 * **Measurement and interpretation are different sentences.** The measured sentence carries the
 * number, the band it was taken over and the convention it was taken under; the interpretation
 * says what that number *may* mean, with the things that also produce it named. So:
 *
 * > Low-mid energy is elevated, 1.3 dB past the +9 dB point … This may contribute to warmth or
 * > to boominess depending on microphone distance and processing.
 *
 * and never "your voice is boomy". Everything a listener would hear is possibilistic, because
 * proximity, compression, microphone response and the room all shape what was measured and no
 * spectrum can separate them.
 *
 * ## Four principles the strings hold to
 * - a voice spectrum does not need to be flat, and high-frequency roll-off is normal: `air`
 *   never recommends a boost, whatever it measures;
 * - the fundamental need not be the strongest harmonic, and here it usually is not;
 * - "harsh", "hard", "boomy" appear only where the evidence is substantial — a reading within
 *   {@link NEAR_THRESHOLD_DB} of the threshold it crossed is phrased as barely across and
 *   recommends nothing at all, because microphone choice alone moves a band by more than that;
 * - a measurement that was not taken produces no prose. Nothing is invented to fill a section.
 *
 * ## `attention` is not one thing
 * H-91 classifies by comparing a measurement with a documented threshold; this module then asks
 * *by how much*. {@link FindingProse.nearThreshold} is the answer, and it changes the words and
 * removes the recommendation. The owner's own take is exactly why: its presence reading crossed
 * by 0.7 dB, which is less than the difference between two microphones.
 */
import { t, type MessageKey, type MessageParams } from "../../i18n";
import { formatNumber } from "../../ui/units";
import {
  EQ_MOVES,
  formatFreqShort,
  HUM_NOTCH_GAIN_DB,
  HUM_NOTCH_Q,
  type EqAction,
  type FindingAction,
} from "../diagnosticsHints";
import { formatCents, noteName, type NoteInfo } from "../notes";
import type { FindingCategory, FindingId, FindingSeverity, VoiceFinding } from "./findings";
import type { VoiceSnapshot } from "./snapshot";
import {
  ACX_NOISE_FLOOR_DBFS,
  HARMONIC_COUNT,
  IMPLAUSIBLE_NOISE_FLOOR_DBFS,
  LOW_PITCH_CONFIDENCE,
  LOW_VOICED_FRACTION,
  NEAR_THRESHOLD_DB,
  NOTABLE_OCTAVE_CORRECTION,
  RUMBLE_WARN_DB,
  SIBILANCE_STRONG_DB,
  SNR_FAIR_DB,
  TONE_ZONES,
} from "./thresholds";

/** One finding, in words. Every field is already rendered through i18n. */
export interface FindingProse {
  id: FindingId;
  category: FindingCategory;
  severity: FindingSeverity;
  /** What the finding is about, with its band where it has one. */
  title: string;
  /** The number, the band and the convention it was measured under. No judgement. */
  measured: string;
  /** What it may mean, and what else produces the same reading. */
  interpretation: string;
  /** `null` unless the measurement justifies a suggestion. */
  recommendation: string | null;
  /** The EQ move or de-esser frequency the recommendation refers to, if any. */
  action: FindingAction | null;
  /** The finding crossed its threshold by no more than {@link NEAR_THRESHOLD_DB}. */
  nearThreshold: boolean;
}

interface Line {
  key: MessageKey;
  params?: MessageParams;
}

interface Draft {
  title: MessageKey;
  measured: Line;
  interpretation: Line[];
  recommendation?: Line | null;
  action?: FindingAction | null;
}

/** "+10.3 dB" / "−1.3 dB" — a level relative to a reference, where the sign is the point. */
function dbRelative(value: number): string {
  return `${formatNumber(value, 1, { signed: true })} dB`;
}

/** "8.0 dB" — a distance between two levels, which is never negative. */
function dbSpan(value: number): string {
  return `${formatNumber(Math.abs(value), 1)} dB`;
}

/** "−30.0 dB" — an absolute level off the curve. */
function dbLevel(value: number): string {
  return `${formatNumber(value, 1)} dB`;
}

function dbfs(value: number): string {
  return `${formatNumber(value, 1)} dBFS`;
}

/** "+9 dB", "−60 dBFS" — a threshold, which is a round number and reads as one. */
function dbThreshold(value: number, unit = "dB"): string {
  return `${formatNumber(value, 0, { signed: unit === "dB" })} ${unit}`;
}

/** "−3 dB", "+2.5 dB" — a suggested move, which never carries a decimal it does not need. */
function dbGain(value: number): string {
  return `${formatNumber(value, Number.isInteger(value) ? 0 : 1, { signed: true })} dB`;
}

/**
 * "198.6 Hz" — a frequency whose arithmetic is on show. {@link formatFreqShort} rounds to whole
 * hertz below 1 kHz, which would make "199 Hz, divided by 2, implies 99 Hz" look like a mistake;
 * where the reader can do the division, the numbers have to survive it.
 */
function freqPrecise(freqHz: number): string {
  return freqHz < 1000 ? `${formatNumber(freqHz, 1)} Hz` : formatFreqShort(freqHz);
}

function percent(fraction: number): string {
  return `${formatNumber(fraction * 100, 0)}%`;
}

/** "B2 −49¢", or an empty string when the frequency has no note. */
function noteText(note: NoteInfo | null): string {
  return note ? `${noteName(note)} ${formatCents(note.cents)}¢` : "";
}

/** An EQ move, in the shape the existing "Add EQ band here" path already applies. */
function eqOf(move: EqAction): FindingAction {
  return { type: "eq", eq: { ...move } };
}

/**
 * The threshold this finding crossed, or `null` when it crossed none (or when crossing is not
 * what its severity means — hum is present or absent, not high or low). This is the number the
 * margin is measured from, so the prose can say *by how much* rather than only *that*.
 */
export function crossedThreshold(finding: VoiceFinding): number | null {
  if (finding.severity !== "attention" && finding.severity !== "significant") {
    return null;
  }
  switch (finding.id) {
    case "body":
      return finding.zone === "above" ? TONE_ZONES.mud.warnHigh : null;
    case "presence":
      return finding.zone === "above" ? TONE_ZONES.presence.high : TONE_ZONES.presence.low;
    case "sibilance":
      return SIBILANCE_STRONG_DB;
    case "rumble":
      return RUMBLE_WARN_DB;
    case "noise_floor":
      return ACX_NOISE_FLOOR_DBFS;
    case "snr":
      return SNR_FAIR_DB;
    default:
      return null;
  }
}

/** How far past its threshold the measurement landed (dB), or `null`. */
export function marginDb(finding: VoiceFinding): number | null {
  const threshold = crossedThreshold(finding);
  return threshold === null ? null : Math.abs(finding.measured.value - threshold);
}

/**
 * `true` when the measurement is within {@link NEAR_THRESHOLD_DB} of the threshold it crossed.
 * The prose then describes it as barely across and offers no correction.
 */
export function isNearThreshold(finding: VoiceFinding): boolean {
  const margin = marginDb(finding);
  return margin !== null && margin <= NEAR_THRESHOLD_DB;
}

function pitchDraft(finding: VoiceFinding, snapshot: VoiceSnapshot): Draft {
  const pitch = snapshot.pitch;
  const detail = finding.detail;
  const semitones = (detail.rangeCents ?? 0) / 100;
  const interpretation: Line[] = [{ key: "explain.finding.f0.interpretation" }];
  if ((detail.confidence ?? 1) < LOW_PITCH_CONFIDENCE) {
    interpretation.push({
      key: "explain.finding.f0.low_confidence",
      params: { confidence: formatNumber(detail.confidence ?? 0, 2) },
    });
  }
  if ((detail.voicedFraction ?? 1) < LOW_VOICED_FRACTION) {
    interpretation.push({ key: "explain.finding.f0.low_voiced" });
  }
  if ((detail.octaveCorrected ?? 0) > NOTABLE_OCTAVE_CORRECTION) {
    interpretation.push({
      key: "explain.finding.f0.octave_corrected",
      params: { corrected: percent(detail.octaveCorrected ?? 0) },
    });
  }
  return {
    title: "explain.finding.f0.title",
    measured: {
      key: "explain.finding.f0.measured",
      params: {
        median: formatFreqShort(finding.measured.value),
        note: noteText(pitch?.note ?? null),
        voiced: percent(detail.voicedFraction ?? 0),
        low: formatNumber(detail.lowHz ?? 0, 0),
        high: formatFreqShort(detail.highHz ?? 0),
        semitones: formatNumber(semitones, 1),
      },
    },
    interpretation,
  };
}

function strongestPeakDraft(finding: VoiceFinding, snapshot: VoiceSnapshot): Draft {
  const peak = snapshot.strongestPeak;
  const detail = finding.detail;
  const level = dbLevel(detail.levelDb ?? 0);
  const freq = freqPrecise(finding.measured.value);
  // H-116: the peak lines up with a harmonic that this take's pitch range cannot separate from
  // its neighbours. That is a weaker, still-true claim than "not a harmonic" — say which harmonic
  // it probably is and that the take cannot be certain, never that it is a resonance instead.
  if (peak && peak.harmonicNumber === null && peak.unresolvedHarmonicNumber !== null) {
    const interpretation: Line[] = [{ key: "explain.finding.strongest_peak.not_fundamental" }];
    if (peak.unresolvedImpliedF0Hz !== null && snapshot.pitch) {
      interpretation.push({
        key: "explain.finding.strongest_peak.likely_unresolved_harmonic",
        params: {
          n: peak.unresolvedHarmonicNumber,
          implied: freqPrecise(peak.unresolvedImpliedF0Hz),
          cents: formatCents(Math.round(peak.unresolvedDeviationCents ?? 0)),
          median: freqPrecise(snapshot.pitch.medianHz),
        },
      });
    }
    return {
      title: "explain.finding.strongest_peak.title",
      measured: {
        key: "explain.finding.strongest_peak.measured_unresolved",
        params: { freq, level },
      },
      interpretation,
    };
  }
  // Only reached once neither a resolvable nor an unresolved harmonic explains the peak — no
  // pitch this speaker used, at any n, implies this frequency.
  if (!peak || peak.harmonicNumber === null) {
    return {
      title: "explain.finding.strongest_peak.title",
      measured: {
        key: "explain.finding.strongest_peak.measured_unrelated",
        params: { freq, level },
      },
      interpretation: [{ key: "explain.finding.strongest_peak.not_fundamental" }],
    };
  }
  if (peak.isFundamental) {
    return {
      title: "explain.finding.strongest_peak.title",
      measured: {
        key: "explain.finding.strongest_peak.measured_fundamental",
        params: { freq, level },
      },
      interpretation: [{ key: "explain.finding.strongest_peak.is_fundamental" }],
    };
  }
  const interpretation: Line[] = [{ key: "explain.finding.strongest_peak.not_fundamental" }];
  if (peak.impliedF0Hz !== null && snapshot.pitch) {
    interpretation.push({
      key: "explain.finding.strongest_peak.two_f0",
      params: {
        n: peak.harmonicNumber,
        implied: freqPrecise(peak.impliedF0Hz),
        cents: formatCents(Math.round(peak.deviationCents ?? 0)),
        // Both figures in this sentence are shown at the same precision: the reader is being
        // invited to compare them, and 99.3 against a rounded 104 would look like sloppiness.
        median: freqPrecise(snapshot.pitch.medianHz),
      },
    });
  }
  return {
    title: "explain.finding.strongest_peak.title",
    measured: {
      key: "explain.finding.strongest_peak.measured_harmonic",
      params: {
        freq,
        level,
        n: peak.harmonicNumber,
        above: dbSpan(detail.aboveFundamentalDb ?? 0),
      },
    },
    interpretation,
  };
}

function harmonicsDraft(_finding: VoiceFinding, snapshot: VoiceSnapshot): Draft {
  const supported = snapshot.harmonics.filter((h) => h.status === "supported");
  const unresolved = snapshot.harmonics.filter((h) => h.status === "unresolved");
  const h1 = snapshot.harmonics.find((h) => h.n === 1);
  const list = supported.map((h) => `H${h.n}`).join(", ");
  const measured: Line =
    supported.length === 0
      ? { key: "explain.finding.harmonics.measured_none" }
      : supported.length === 1
        ? {
            key: "explain.finding.harmonics.measured_supported_one",
            params: { top: HARMONIC_COUNT, list },
          }
        : {
            key: "explain.finding.harmonics.measured_supported",
            params: { supported: supported.length, top: HARMONIC_COUNT, list },
          };
  const interpretation: Line[] = [];
  if (unresolved.length > 0 && snapshot.pitch) {
    interpretation.push({
      key: "explain.finding.harmonics.unresolved",
      params: {
        first: unresolved[0]!.n,
        low: formatFreqShort(snapshot.pitch.lowHz),
        high: formatFreqShort(snapshot.pitch.highHz),
      },
    });
  } else {
    interpretation.push({ key: "explain.finding.harmonics.all_resolved" });
  }
  if (h1 && h1.status === "weak") {
    interpretation.push({ key: "explain.finding.harmonics.weak_fundamental" });
  }
  return { title: "explain.finding.harmonics.title", measured, interpretation };
}

function bodyDraft(finding: VoiceFinding): Draft {
  const margin = marginDb(finding);
  const near = isNearThreshold(finding);
  const measured: Line = {
    key: "explain.finding.body.measured",
    params: { value: dbRelative(finding.measured.value) },
  };
  if (finding.severity === "good") {
    return {
      title: "explain.finding.body.title",
      measured,
      interpretation: [{ key: "explain.finding.body.in_range" }],
    };
  }
  if (finding.severity === "info") {
    return {
      title: "explain.finding.body.title",
      measured,
      interpretation: [
        { key: finding.zone === "above" ? "explain.finding.body.info_high" : "explain.finding.body.info_low" },
      ],
    };
  }
  const params = {
    margin: dbSpan(margin ?? 0),
    warn: dbThreshold(TONE_ZONES.mud.warnHigh),
  };
  const key: MessageKey = near
    ? "explain.finding.body.high_near"
    : finding.severity === "significant"
      ? "explain.finding.body.high_significant"
      : "explain.finding.body.high";
  return {
    title: "explain.finding.body.title",
    measured,
    interpretation: [{ key, params }],
    // Barely across is not a reason to change anything: the source check is the whole advice.
    recommendation: near
      ? null
      : {
          key: "explain.finding.body.rec",
          params: {
            gain: dbGain(EQ_MOVES.mudCut.gainDb),
            freq: formatFreqShort(EQ_MOVES.mudCut.freqHz),
          },
        },
    action: near ? null : eqOf(EQ_MOVES.mudCut),
  };
}

function presenceDraft(finding: VoiceFinding): Draft {
  const margin = marginDb(finding);
  const near = isNearThreshold(finding);
  const measured: Line = {
    key: "explain.finding.presence.measured",
    params: {
      value: dbRelative(finding.measured.value),
      low: dbThreshold(TONE_ZONES.presence.low),
      high: dbThreshold(TONE_ZONES.presence.high),
    },
  };
  if (finding.severity === "good" || finding.severity === "info") {
    return {
      title: "explain.finding.presence.title",
      measured,
      interpretation: [{ key: "explain.finding.presence.in_range" }],
    };
  }
  const params = { margin: dbSpan(margin ?? 0) };
  if (finding.zone === "below") {
    return {
      title: "explain.finding.presence.title",
      measured,
      interpretation: [{ key: "explain.finding.presence.low", params }],
      recommendation: {
        key: "explain.finding.presence.rec_low",
        params: {
          gain: dbGain(EQ_MOVES.presenceBoost.gainDb),
          freq: formatFreqShort(EQ_MOVES.presenceBoost.freqHz),
        },
      },
      action: eqOf(EQ_MOVES.presenceBoost),
    };
  }
  const key: MessageKey = near
    ? "explain.finding.presence.high_near"
    : finding.severity === "significant"
      ? "explain.finding.presence.high_significant"
      : "explain.finding.presence.high";
  return {
    title: "explain.finding.presence.title",
    measured,
    interpretation: [{ key, params }],
    recommendation: near
      ? null
      : {
          key: "explain.finding.presence.rec_high",
          params: {
            gain: dbGain(EQ_MOVES.presenceCut.gainDb),
            freq: formatFreqShort(EQ_MOVES.presenceCut.freqHz),
          },
        },
    action: near ? null : eqOf(EQ_MOVES.presenceCut),
  };
}

/** Air never recommends anything: a voice spectrum is supposed to roll off up here. */
function airDraft(finding: VoiceFinding): Draft {
  const key: MessageKey =
    finding.zone === "below"
      ? "explain.finding.air.low"
      : finding.zone === "above"
        ? "explain.finding.air.high"
        : "explain.finding.air.in_range";
  return {
    title: "explain.finding.air.title",
    measured: {
      key: "explain.finding.air.measured",
      params: { value: dbRelative(finding.measured.value) },
    },
    interpretation: [{ key }],
  };
}

function sibilanceDraft(finding: VoiceFinding): Draft {
  const centreHz = finding.detail.centreHz ?? 0;
  const measured: Line = {
    key: "explain.finding.sibilance.measured",
    params: {
      value: dbRelative(finding.measured.value),
      freq: formatFreqShort(centreHz),
    },
  };
  if (finding.severity === "good") {
    return {
      title: "explain.finding.sibilance.title",
      measured,
      interpretation: [{ key: "explain.finding.sibilance.in_range" }],
    };
  }
  if (finding.severity === "info") {
    return {
      title: "explain.finding.sibilance.title",
      measured,
      interpretation: [{ key: "explain.finding.sibilance.moderate" }],
    };
  }
  return {
    title: "explain.finding.sibilance.title",
    measured,
    interpretation: [
      { key: "explain.finding.sibilance.strong", params: { margin: dbSpan(marginDb(finding) ?? 0) } },
    ],
    recommendation: {
      key: "explain.finding.sibilance.rec",
      params: { freq: formatFreqShort(centreHz) },
    },
    // There is no de-esser module yet (SPEC-007 §8.10), so the measured centre is offered to copy.
    action: { type: "copy", freqHz: centreHz },
  };
}

function humDraft(finding: VoiceFinding): Draft {
  if (finding.zone === "absent") {
    return {
      title: "explain.finding.hum.title",
      measured: { key: "explain.finding.hum.measured_absent" },
      interpretation: [{ key: "explain.finding.hum.absent" }],
    };
  }
  const strongestHz = finding.detail.strongestHz ?? 0;
  return {
    title: "explain.finding.hum.title",
    measured: {
      key: "explain.finding.hum.measured_present",
      params: {
        freq: formatFreqShort(strongestHz),
        prominence: dbSpan(finding.measured.value),
        count: finding.detail.harmonicCount ?? 0,
        mains: formatFreqShort(finding.detail.mainsHz ?? 0),
      },
    },
    interpretation: [{ key: "explain.finding.hum.present" }],
    recommendation: {
      key: "explain.finding.hum.rec",
      params: { freq: formatFreqShort(strongestHz) },
    },
    action: eqOf({ kind: "notch", freqHz: strongestHz, gainDb: HUM_NOTCH_GAIN_DB, q: HUM_NOTCH_Q }),
  };
}

function rumbleDraft(finding: VoiceFinding): Draft {
  const measured: Line = {
    key: "explain.finding.rumble.measured",
    params: { value: dbRelative(finding.measured.value) },
  };
  if (finding.severity === "good" || finding.severity === "info") {
    return {
      title: "explain.finding.rumble.title",
      measured,
      interpretation: [{ key: "explain.finding.rumble.in_range" }],
    };
  }
  return {
    title: "explain.finding.rumble.title",
    measured,
    interpretation: [
      { key: "explain.finding.rumble.high", params: { margin: dbSpan(marginDb(finding) ?? 0) } },
    ],
    recommendation: {
      key: "explain.finding.rumble.rec",
      params: { freq: formatFreqShort(EQ_MOVES.rumbleHighPass.freqHz) },
    },
    action: eqOf(EQ_MOVES.rumbleHighPass),
  };
}

/** `true` when a floor is lower than a microphone, its preamp and a room produce together. */
export function floorLooksProcessed(noiseFloorDbfs: number | null): boolean {
  return noiseFloorDbfs !== null && noiseFloorDbfs < IMPLAUSIBLE_NOISE_FLOOR_DBFS;
}

function noiseFloorDraft(finding: VoiceFinding): Draft {
  const value = finding.measured.value;
  const measured: Line = {
    key: "explain.finding.noise_floor.measured",
    params: { value: dbfs(value) },
  };
  const limit = dbThreshold(ACX_NOISE_FLOOR_DBFS, "dBFS");
  if (finding.severity === "attention" || finding.severity === "significant") {
    return {
      title: "explain.finding.noise_floor.title",
      measured,
      interpretation: [
        {
          key: "explain.finding.noise_floor.high",
          params: { margin: dbSpan(marginDb(finding) ?? 0), limit },
        },
      ],
      recommendation: { key: "explain.finding.noise_floor.rec_high" },
    };
  }
  if (floorLooksProcessed(value)) {
    return {
      title: "explain.finding.noise_floor.title",
      measured,
      interpretation: [
        {
          key: "explain.finding.noise_floor.implausible",
          params: { margin: dbSpan(value - ACX_NOISE_FLOOR_DBFS), limit },
        },
      ],
    };
  }
  return {
    title: "explain.finding.noise_floor.title",
    measured,
    interpretation: [{ key: "explain.finding.noise_floor.ok", params: { limit } }],
  };
}

function snrDraft(finding: VoiceFinding): Draft {
  const detail = finding.detail;
  const measured: Line = {
    key: "explain.finding.snr.measured",
    params: {
      value: dbSpan(finding.measured.value),
      active: dbfs(detail.activeLevelDbfs ?? 0),
      floor: dbfs(detail.noiseFloorDbfs ?? 0),
    },
  };
  if (finding.severity === "attention" || finding.severity === "significant") {
    return {
      title: "explain.finding.snr.title",
      measured,
      interpretation: [
        {
          key: "explain.finding.snr.low",
          params: { margin: dbSpan(marginDb(finding) ?? 0), fair: dbThreshold(SNR_FAIR_DB) },
        },
      ],
      recommendation: { key: "explain.finding.snr.rec_low" },
    };
  }
  if (finding.severity === "info") {
    return {
      title: "explain.finding.snr.title",
      measured,
      interpretation: [{ key: "explain.finding.snr.fair" }],
    };
  }
  // A wide SNR measured against a floor that was probably gated is a property of the file, and
  // the words must not congratulate anyone for it.
  return {
    title: "explain.finding.snr.title",
    measured,
    interpretation: [
      {
        key: floorLooksProcessed(detail.noiseFloorDbfs ?? null)
          ? "explain.finding.snr.good_processed"
          : "explain.finding.snr.good",
      },
    ],
  };
}

function draftFor(finding: VoiceFinding, snapshot: VoiceSnapshot): Draft {
  switch (finding.id) {
    case "f0":
      return pitchDraft(finding, snapshot);
    case "strongest_peak":
      return strongestPeakDraft(finding, snapshot);
    case "harmonics":
      return harmonicsDraft(finding, snapshot);
    case "body":
      return bodyDraft(finding);
    case "presence":
      return presenceDraft(finding);
    case "air":
      return airDraft(finding);
    case "sibilance":
      return sibilanceDraft(finding);
    case "hum":
      return humDraft(finding);
    case "rumble":
      return rumbleDraft(finding);
    case "noise_floor":
      return noiseFloorDraft(finding);
    case "snr":
      return snrDraft(finding);
  }
}

function render(line: Line): string {
  return t(line.key, line.params);
}

/** The short name of a finding, for a list or a heading. */
export function shortName(id: FindingId): string {
  return t(`explain.finding.${id}.short` as MessageKey);
}

/** One finding, in words. */
export function proseOf(finding: VoiceFinding, snapshot: VoiceSnapshot): FindingProse {
  const draft = draftFor(finding, snapshot);
  return {
    id: finding.id,
    category: finding.category,
    severity: finding.severity,
    title: t(draft.title),
    measured: render(draft.measured),
    interpretation: draft.interpretation.map(render).join(" "),
    recommendation: draft.recommendation ? render(draft.recommendation) : null,
    action: draft.action ?? null,
    nearThreshold: isNearThreshold(finding),
  };
}

/** Every finding of the snapshot, in words, in the snapshot's own order. */
export function explainFindings(snapshot: VoiceSnapshot): FindingProse[] {
  return snapshot.findings.map((finding) => proseOf(finding, snapshot));
}

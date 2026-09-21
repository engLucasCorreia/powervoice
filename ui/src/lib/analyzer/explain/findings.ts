/**
 * The findings model behind *Explain My Voice* (H-91 §5–§6).
 *
 * Three things are deliberately kept apart, because conflating them is how this kind of feature
 * loses an engineer's trust:
 *
 * 1. **measurement** — `measured` (a number and its unit) plus `detail` (the other measured
 *    numbers the same finding rests on). Nothing here is rounded, named or judged.
 * 2. **classification** — `zone` (where the measurement fell relative to the thresholds in
 *    `thresholds.ts`) and `severity`. Both are decided *only* by comparing a measured value with
 *    a documented threshold.
 * 3. **interpretation** — the words. Not here: H-94 writes them from `id`, `zone` and the
 *    measured numbers.
 *
 * ## Severity discipline (H-91 §6)
 * `good` and `info` are both "nothing is wrong": `good` means the measurement landed in the
 * healthy zone, `info` means it is outside it in a way that describes the voice rather than
 * faulting it — a deep voice with a lot of 200–500 Hz is `info: strong body`, and stays `info`
 * until SPEC-007 §8.10's actual warn threshold is crossed. `attention` means a documented
 * threshold *was* crossed; `significant` means crossed by more than
 * {@link SIGNIFICANT_MARGIN_DB}. High-frequency roll-off never escalates at all — it is normal
 * for a voice, and "air is low" is a description, not a defect.
 */
import type { VoiceReportDto } from "../../ipc/bindings";
import type { FundamentalChoice, HarmonicMeasurement, PeakRelation } from "./harmonics";
import {
  ACX_NOISE_FLOOR_DBFS,
  BASE_PRIORITY,
  HUM_SIGNIFICANT_PROMINENCE_DB,
  LOW_PITCH_CONFIDENCE,
  LOW_VOICED_FRACTION,
  NOTABLE_OCTAVE_CORRECTION,
  RUMBLE_WARN_DB,
  SEVERITY_PRIORITY,
  SIBILANCE_MODERATE_DB,
  SIBILANCE_STRONG_DB,
  SIGNIFICANT_MARGIN_DB,
  SNR_FAIR_DB,
  SNR_GOOD_DB,
  TONE_ZONES,
} from "./thresholds";

/** SPEC-007 §8.7 band edges the findings anchor to (Hz). */
export const BODY_BAND_HZ: [number, number] = [200, 500];
export const PRESENCE_BAND_HZ: [number, number] = [2000, 5000];
export const AIR_BAND_HZ: [number, number] = [10_000, 16_000];
export const SIBILANCE_BAND_HZ: [number, number] = [4000, 10_000];
export const RUMBLE_BAND_HZ: [number, number] = [20, 80];

export type FindingId = keyof typeof BASE_PRIORITY;

export type FindingSeverity = "good" | "info" | "attention" | "significant";

/** What kind of measurement the finding is about — H-92 groups the annotations by this. */
export type FindingCategory = "pitch" | "harmonics" | "tone" | "sibilance" | "noise";

/** Where the measurement fell relative to its thresholds. */
export type FindingZone = "below" | "in_range" | "above" | "present" | "absent";

export type MeasurementUnit = "hz" | "db" | "dbfs" | "cents" | "count" | "fraction";

/** What the finding points at on the graph. The anchor is a **measured** frequency or a band
 * edge — never a convenient spot (H-93 may move the label, never the anchor). */
export type FindingAnchor =
  | { kind: "frequency"; freqHz: number }
  | { kind: "band"; lowHz: number; highHz: number }
  | { kind: "none" };

export interface Measurement {
  value: number;
  unit: MeasurementUnit;
}

export interface VoiceFinding {
  id: FindingId;
  category: FindingCategory;
  severity: FindingSeverity;
  zone: FindingZone;
  anchor: FindingAnchor;
  /** The number this finding is about. */
  measured: Measurement;
  /** The other measured numbers it rests on (`null` = that one was not measurable). */
  detail: Record<string, number | null>;
  /** Highest first; H-93 drops the lowest when space runs out. */
  priority: number;
}

function priorityOf(id: FindingId, severity: FindingSeverity): number {
  return BASE_PRIORITY[id] + SEVERITY_PRIORITY[severity];
}

function finding(
  id: FindingId,
  category: FindingCategory,
  severity: FindingSeverity,
  zone: FindingZone,
  anchor: FindingAnchor,
  measured: Measurement,
  detail: Record<string, number | null> = {},
): VoiceFinding {
  return { id, category, severity, zone, anchor, measured, detail, priority: priorityOf(id, severity) };
}

/**
 * The severity of a value that may be too low or too high. `info` is used for the zone that
 * merely describes the voice; `attention` starts at `warnLow`/`warnHigh`, and `significant`
 * {@link SIGNIFICANT_MARGIN_DB} past it.
 */
function twoSided(
  value: number,
  bounds: { low: number; high: number; warnLow?: number; warnHigh?: number },
): { severity: FindingSeverity; zone: FindingZone } {
  const warnHigh = bounds.warnHigh ?? bounds.high;
  const warnLow = bounds.warnLow ?? bounds.low;
  if (value > warnHigh) {
    return {
      severity: value > warnHigh + SIGNIFICANT_MARGIN_DB ? "significant" : "attention",
      zone: "above",
    };
  }
  if (value < warnLow) {
    return {
      severity: value < warnLow - SIGNIFICANT_MARGIN_DB ? "significant" : "attention",
      zone: "below",
    };
  }
  if (value > bounds.high) {
    return { severity: "info", zone: "above" };
  }
  if (value < bounds.low) {
    return { severity: "info", zone: "below" };
  }
  return { severity: "good", zone: "in_range" };
}

/** The severity of a value where only "too high" is a problem. */
function oneSidedHigh(
  value: number,
  infoAbove: number | null,
  warnAbove: number,
): { severity: FindingSeverity; zone: FindingZone } {
  if (value > warnAbove + SIGNIFICANT_MARGIN_DB) {
    return { severity: "significant", zone: "above" };
  }
  if (value > warnAbove) {
    return { severity: "attention", zone: "above" };
  }
  if (infoAbove !== null && value > infoAbove) {
    return { severity: "info", zone: "above" };
  }
  return { severity: "good", zone: "in_range" };
}

function pitchFindings(
  report: VoiceReportDto,
  fundamental: FundamentalChoice | null,
  harmonics: HarmonicMeasurement[],
  strongest: PeakRelation | null,
): VoiceFinding[] {
  const f0 = report.f0;
  if (!f0 || !fundamental) {
    return [];
  }
  const out: VoiceFinding[] = [];
  // The pitch reading itself: a measurement, never a verdict — SPEC-007 §8.6 is explicit that
  // F0 is descriptive and carries no voice-type label.
  out.push(
    finding(
      "f0",
      "pitch",
      f0.confidence < LOW_PITCH_CONFIDENCE || f0.voiced_fraction < LOW_VOICED_FRACTION
        ? "info"
        : "good",
      "in_range",
      { kind: "frequency", freqHz: fundamental.fundamentalHz },
      { value: fundamental.fundamentalHz, unit: "hz" },
      {
        trackedMedianHz: fundamental.trackedHz,
        lowHz: f0.low_hz,
        highHz: f0.high_hz,
        rangeCents: 1200 * Math.log2(Math.max(f0.high_hz, 1e-9) / Math.max(f0.low_hz, 1e-9)),
        confidence: f0.confidence,
        octaveCorrected: f0.octave_corrected,
        voicedFraction: f0.voiced_fraction,
        octaveMoved: fundamental.ratio === 1 ? 0 : 1,
        octaveChecked: fundamental.checked ? 1 : 0,
        correctionNotable: f0.octave_corrected > NOTABLE_OCTAVE_CORRECTION ? 1 : 0,
      },
    ),
  );

  if (strongest) {
    // H-91 §3: the single most educational thing the feature does. Still `info` — a voice whose
    // second harmonic is its loudest partial is normal, not a fault.
    out.push(
      finding(
        "strongest_peak",
        "harmonics",
        "info",
        strongest.isFundamental ? "in_range" : "above",
        { kind: "frequency", freqHz: strongest.freqHz },
        { value: strongest.freqHz, unit: "hz" },
        {
          levelDb: strongest.levelDb,
          prominenceDb: strongest.prominenceDb,
          harmonicNumber: strongest.harmonicNumber,
          impliedF0Hz: strongest.impliedF0Hz,
          deviationCents: strongest.deviationCents,
          aboveFundamentalDb: strongest.aboveFundamentalDb,
          unresolvedHarmonicNumber: strongest.unresolvedHarmonicNumber,
          unresolvedImpliedF0Hz: strongest.unresolvedImpliedF0Hz,
          unresolvedDeviationCents: strongest.unresolvedDeviationCents,
        },
      ),
    );
  }

  const supported = harmonics.filter((h) => h.status === "supported");
  const resolvable = harmonics.filter((h) => h.status !== "unresolved");
  out.push(
    finding(
      "harmonics",
      "harmonics",
      "info",
      supported.length > 0 ? "present" : "absent",
      supported.length > 0
        ? {
            kind: "band",
            lowHz: supported[0]!.bandHz[0],
            highHz: supported[supported.length - 1]!.bandHz[1],
          }
        : { kind: "none" },
      { value: supported.length, unit: "count" },
      {
        resolvable: resolvable.length,
        highestSupported: supported.length > 0 ? supported[supported.length - 1]!.n : null,
      },
    ),
  );
  return out;
}

function toneFindings(report: VoiceReportDto): VoiceFinding[] {
  const tone = report.tone;
  if (!tone) {
    return [];
  }
  const out: VoiceFinding[] = [];
  const body = twoSided(tone.mud_db, {
    low: TONE_ZONES.mud.low,
    high: TONE_ZONES.mud.high,
    warnHigh: TONE_ZONES.mud.warnHigh,
    // There is no "too thin to be allowed": below the low bound it stays descriptive.
    warnLow: -Infinity,
  });
  out.push(
    finding("body", "tone", body.severity, body.zone, { kind: "band", lowHz: BODY_BAND_HZ[0], highHz: BODY_BAND_HZ[1] }, { value: tone.mud_db, unit: "db" }),
  );
  if (tone.presence_db !== null) {
    const presence = twoSided(tone.presence_db, TONE_ZONES.presence);
    out.push(
      finding("presence", "tone", presence.severity, presence.zone, { kind: "band", lowHz: PRESENCE_BAND_HZ[0], highHz: PRESENCE_BAND_HZ[1] }, { value: tone.presence_db, unit: "db" }),
    );
  }
  if (tone.air_db !== null) {
    // Never escalates: a voice spectrum rolls off, and that is not a defect (H-94's principle,
    // applied here so the severity can't contradict the prose).
    const zone: FindingZone =
      tone.air_db < TONE_ZONES.air.low ? "below" : tone.air_db > TONE_ZONES.air.high ? "above" : "in_range";
    out.push(
      finding("air", "tone", zone === "in_range" ? "good" : "info", zone, { kind: "band", lowHz: AIR_BAND_HZ[0], highHz: AIR_BAND_HZ[1] }, { value: tone.air_db, unit: "db" }),
    );
  }
  return out;
}

function noiseFindings(report: VoiceReportDto): VoiceFinding[] {
  const out: VoiceFinding[] = [];
  if (report.sibilance) {
    const s = report.sibilance;
    const { severity, zone } = oneSidedHigh(s.ratio_db, SIBILANCE_MODERATE_DB, SIBILANCE_STRONG_DB);
    out.push(
      finding("sibilance", "sibilance", severity, zone, { kind: "frequency", freqHz: s.centre_hz }, { value: s.ratio_db, unit: "db" }, {
        centreHz: s.centre_hz,
        bandLowHz: SIBILANCE_BAND_HZ[0],
        bandHighHz: SIBILANCE_BAND_HZ[1],
      }),
    );
  }
  if (report.hum) {
    const h = report.hum;
    out.push(
      finding(
        "hum",
        "noise",
        h.prominence_db >= HUM_SIGNIFICANT_PROMINENCE_DB ? "significant" : "attention",
        "present",
        { kind: "frequency", freqHz: h.strongest_hz },
        { value: h.prominence_db, unit: "db" },
        {
          mainsHz: h.mains_hz,
          strongestHz: h.strongest_hz,
          levelDb: h.level_db,
          harmonicCount: h.harmonics.length,
        },
      ),
    );
  } else if (report.noise_floor_dbfs !== null) {
    out.push(finding("hum", "noise", "good", "absent", { kind: "none" }, { value: 0, unit: "db" }));
  }
  if (report.rumble_db !== null) {
    const { severity, zone } = oneSidedHigh(report.rumble_db, null, RUMBLE_WARN_DB);
    out.push(
      finding("rumble", "noise", severity, zone, { kind: "band", lowHz: RUMBLE_BAND_HZ[0], highHz: RUMBLE_BAND_HZ[1] }, { value: report.rumble_db, unit: "db" }),
    );
  }
  if (report.noise_floor_dbfs !== null) {
    const { severity, zone } = oneSidedHigh(report.noise_floor_dbfs, null, ACX_NOISE_FLOOR_DBFS);
    out.push(
      // Broadband time-domain RMS (SPEC-007 §8.8), never an FFT-bin floor — `detail` says so
      // explicitly so the prose can't quietly relabel it.
      finding("noise_floor", "noise", severity, zone, { kind: "none" }, { value: report.noise_floor_dbfs, unit: "dbfs" }, {
        activeLevelDbfs: report.active_level_dbfs,
        broadbandRms: 1,
      }),
    );
  }
  if (report.snr_db !== null) {
    const snr = report.snr_db;
    const severity: FindingSeverity =
      snr < SNR_FAIR_DB - SIGNIFICANT_MARGIN_DB
        ? "significant"
        : snr < SNR_FAIR_DB
          ? "attention"
          : snr < SNR_GOOD_DB
            ? "info"
            : "good";
    out.push(
      finding("snr", "noise", severity, snr < SNR_GOOD_DB ? "below" : "in_range", { kind: "none" }, { value: snr, unit: "db" }, {
        noiseFloorDbfs: report.noise_floor_dbfs,
        activeLevelDbfs: report.active_level_dbfs,
      }),
    );
  }
  return out;
}

/**
 * Every finding the measurements support, highest `priority` first. Nothing is invented to fill
 * a section: a measurement that was not available produces no finding at all (H-94's
 * clean-recording rule starts here).
 */
export function buildFindings(input: {
  report: VoiceReportDto;
  fundamental: FundamentalChoice | null;
  harmonics: HarmonicMeasurement[];
  strongestPeak: PeakRelation | null;
}): VoiceFinding[] {
  const out = [
    ...pitchFindings(input.report, input.fundamental, input.harmonics, input.strongestPeak),
    ...toneFindings(input.report),
    ...noiseFindings(input.report),
  ];
  return out.sort((a, b) => b.priority - a.priority || a.id.localeCompare(b.id));
}

/** The findings that crossed a threshold, worst first — what a summary leads with. */
export function findingsNeedingAttention(findings: VoiceFinding[]): VoiceFinding[] {
  const rank: Record<FindingSeverity, number> = { significant: 3, attention: 2, info: 1, good: 0 };
  return findings
    .filter((f) => f.severity === "attention" || f.severity === "significant")
    .sort((a, b) => rank[b.severity] - rank[a.severity] || b.priority - a.priority);
}

import { describe, expect, it } from "vitest";

import type { VoiceReportDto } from "../../ipc/bindings";
import { buildFindings, findingsNeedingAttention, type FindingId, type VoiceFinding } from "./findings";
import type { FundamentalChoice } from "./harmonics";
import {
  ACX_NOISE_FLOOR_DBFS,
  HUM_SIGNIFICANT_PROMINENCE_DB,
  RUMBLE_WARN_DB,
  SIBILANCE_MODERATE_DB,
  SIBILANCE_STRONG_DB,
  SIGNIFICANT_MARGIN_DB,
  SNR_FAIR_DB,
  SNR_GOOD_DB,
  TONE_ZONES,
} from "./thresholds";
import { balancedReport } from "./voiceFixtures";

/** Smallest step that is unambiguously "past" a threshold. */
const EPS = 0.01;

const FUNDAMENTAL: FundamentalChoice = {
  trackedHz: 120,
  fundamentalHz: 120,
  ratio: 1,
  checked: true,
  subHarmonics: [],
};

function findings(overrides: Partial<VoiceReportDto>): VoiceFinding[] {
  return buildFindings({
    report: balancedReport(overrides),
    fundamental: FUNDAMENTAL,
    harmonics: [],
    strongestPeak: null,
  });
}

function severityOf(id: FindingId, overrides: Partial<VoiceReportDto>): string | undefined {
  return findings(overrides).find((f) => f.id === id)?.severity;
}

function zoneOf(id: FindingId, overrides: Partial<VoiceReportDto>): string | undefined {
  return findings(overrides).find((f) => f.id === id)?.zone;
}

function tone(patch: Partial<NonNullable<VoiceReportDto["tone"]>>): Partial<VoiceReportDto> {
  return { tone: { ...balancedReport().tone!, ...patch } };
}

describe("severity at the thresholds", () => {
  it("low-mid energy: describes, then escalates only at the warn threshold", () => {
    expect(severityOf("body", tone({ mud_db: TONE_ZONES.mud.high }))).toBe("good");
    // H-91 §6: a deep voice with strong 200–500 Hz is information, not a problem…
    expect(severityOf("body", tone({ mud_db: TONE_ZONES.mud.high + EPS }))).toBe("info");
    expect(severityOf("body", tone({ mud_db: TONE_ZONES.mud.warnHigh }))).toBe("info");
    // …until SPEC-007 §8.10's own warn threshold is actually crossed.
    expect(severityOf("body", tone({ mud_db: TONE_ZONES.mud.warnHigh + EPS }))).toBe("attention");
    expect(
      severityOf("body", tone({ mud_db: TONE_ZONES.mud.warnHigh + SIGNIFICANT_MARGIN_DB })),
    ).toBe("attention");
    expect(
      severityOf("body", tone({ mud_db: TONE_ZONES.mud.warnHigh + SIGNIFICANT_MARGIN_DB + EPS })),
    ).toBe("significant");
    // A lean voice is described, never faulted.
    expect(severityOf("body", tone({ mud_db: TONE_ZONES.mud.low - EPS }))).toBe("info");
    expect(severityOf("body", tone({ mud_db: TONE_ZONES.mud.low - 40 }))).toBe("info");
    expect(zoneOf("body", tone({ mud_db: TONE_ZONES.mud.low - EPS }))).toBe("below");
  });

  it("presence: both edges escalate", () => {
    expect(severityOf("presence", tone({ presence_db: TONE_ZONES.presence.low }))).toBe("good");
    expect(severityOf("presence", tone({ presence_db: TONE_ZONES.presence.low - EPS }))).toBe("attention");
    expect(
      severityOf("presence", tone({ presence_db: TONE_ZONES.presence.low - SIGNIFICANT_MARGIN_DB - EPS })),
    ).toBe("significant");
    expect(severityOf("presence", tone({ presence_db: TONE_ZONES.presence.high }))).toBe("good");
    expect(severityOf("presence", tone({ presence_db: TONE_ZONES.presence.high + EPS }))).toBe("attention");
    expect(
      severityOf("presence", tone({ presence_db: TONE_ZONES.presence.high + SIGNIFICANT_MARGIN_DB + EPS })),
    ).toBe("significant");
  });

  it("air never escalates — a voice spectrum is supposed to roll off", () => {
    for (const air of [TONE_ZONES.air.low - EPS, -60, TONE_ZONES.air.high + EPS, 0]) {
      const severity = severityOf("air", tone({ air_db: air }));
      expect(severity === "info" || severity === "good").toBe(true);
    }
    expect(severityOf("air", tone({ air_db: TONE_ZONES.air.low + 1 }))).toBe("good");
    expect(zoneOf("air", tone({ air_db: TONE_ZONES.air.low - EPS }))).toBe("below");
  });

  it("sibilance", () => {
    const sib = (ratio_db: number) => ({ sibilance: { ratio_db, centre_hz: 6800 } });
    expect(severityOf("sibilance", sib(SIBILANCE_MODERATE_DB))).toBe("good");
    expect(severityOf("sibilance", sib(SIBILANCE_MODERATE_DB + EPS))).toBe("info");
    expect(severityOf("sibilance", sib(SIBILANCE_STRONG_DB))).toBe("info");
    expect(severityOf("sibilance", sib(SIBILANCE_STRONG_DB + EPS))).toBe("attention");
    expect(severityOf("sibilance", sib(SIBILANCE_STRONG_DB + SIGNIFICANT_MARGIN_DB + EPS))).toBe(
      "significant",
    );
    // The anchor is the measured de-esser centre, never a default.
    const finding = findings(sib(SIBILANCE_STRONG_DB + 1)).find((f) => f.id === "sibilance")!;
    expect(finding.anchor).toEqual({ kind: "frequency", freqHz: 6800 });
    expect(finding.detail.centreHz).toBe(6800);
  });

  it("rumble, noise floor and SNR", () => {
    expect(severityOf("rumble", { rumble_db: RUMBLE_WARN_DB })).toBe("good");
    expect(severityOf("rumble", { rumble_db: RUMBLE_WARN_DB + EPS })).toBe("attention");
    expect(severityOf("rumble", { rumble_db: RUMBLE_WARN_DB + SIGNIFICANT_MARGIN_DB + EPS })).toBe(
      "significant",
    );

    expect(severityOf("noise_floor", { noise_floor_dbfs: ACX_NOISE_FLOOR_DBFS })).toBe("good");
    expect(severityOf("noise_floor", { noise_floor_dbfs: ACX_NOISE_FLOOR_DBFS + EPS })).toBe("attention");
    expect(
      severityOf("noise_floor", { noise_floor_dbfs: ACX_NOISE_FLOOR_DBFS + SIGNIFICANT_MARGIN_DB + EPS }),
    ).toBe("significant");

    expect(severityOf("snr", { snr_db: SNR_GOOD_DB })).toBe("good");
    expect(severityOf("snr", { snr_db: SNR_GOOD_DB - EPS })).toBe("info");
    expect(severityOf("snr", { snr_db: SNR_FAIR_DB - EPS })).toBe("attention");
    expect(severityOf("snr", { snr_db: SNR_FAIR_DB - SIGNIFICANT_MARGIN_DB - EPS })).toBe("significant");
  });

  it("the noise floor is labelled as the broadband RMS measurement it is", () => {
    const finding = findings({ noise_floor_dbfs: -70 }).find((f) => f.id === "noise_floor")!;
    expect(finding.measured.unit).toBe("dbfs");
    expect(finding.detail.broadbandRms).toBe(1);
  });

  it("hum", () => {
    const hum = (prominence_db: number) => ({
      hum: {
        mains_hz: 50,
        harmonics: [1, 2, 3],
        strongest_hz: 100.1,
        prominence_db,
        level_db: -58,
      },
    });
    expect(severityOf("hum", hum(HUM_SIGNIFICANT_PROMINENCE_DB - EPS))).toBe("attention");
    expect(severityOf("hum", hum(HUM_SIGNIFICANT_PROMINENCE_DB))).toBe("significant");
    // The notch goes at the measured line, not at the nominal mains frequency.
    expect(findings(hum(14)).find((f) => f.id === "hum")!.anchor).toEqual({
      kind: "frequency",
      freqHz: 100.1,
    });
    expect(severityOf("hum", { hum: null })).toBe("good");
  });
});

describe("the findings model", () => {
  it("invents nothing for a clean recording", () => {
    const clean = findings({});
    expect(findingsNeedingAttention(clean)).toHaveLength(0);
    for (const finding of clean) {
      expect(["good", "info"]).toContain(finding.severity);
    }
  });

  it("produces no finding for a measurement that was not available", () => {
    const empty = buildFindings({
      report: {
        f0: null,
        tone: null,
        sibilance: null,
        hum: null,
        rumble_db: null,
        noise_floor_dbfs: null,
        active_level_dbfs: null,
        snr_db: null,
        span_s: 0,
      },
      fundamental: null,
      harmonics: [],
      strongestPeak: null,
    });
    expect(empty).toHaveLength(0);
  });

  it("keeps measurement, classification and interpretation apart", () => {
    const mud = TONE_ZONES.mud.warnHigh + 2;
    const finding = findings(tone({ mud_db: mud })).find((f) => f.id === "body")!;
    // The measurement is the raw number, not a rounded or reworded one…
    expect(finding.measured).toEqual({ value: mud, unit: "db" });
    // …the classification is only where it fell…
    expect(finding.zone).toBe("above");
    expect(finding.severity).toBe("attention");
    // …and no prose is anywhere in the model (H-94 owns the words).
    expect(Object.values(finding)).not.toContainEqual(expect.stringContaining(" "));
  });

  it("orders by priority so a crossed threshold outranks a quieter neighbour", () => {
    const loud = findings({ sibilance: { ratio_db: SIBILANCE_STRONG_DB + 8, centre_hz: 7100 } });
    const ids = loud.map((f) => f.id);
    expect(ids.indexOf("sibilance")).toBeLessThan(ids.indexOf("f0"));
    // Priorities are strictly ordered, so H-93 can drop from the end.
    for (let i = 1; i < loud.length; i++) {
      expect(loud[i - 1]!.priority).toBeGreaterThanOrEqual(loud[i]!.priority);
    }
    // A calm sibilance reading falls back behind the pitch reading.
    const calm = findings({});
    const calmIds = calm.map((f) => f.id);
    expect(calmIds.indexOf("f0")).toBeLessThan(calmIds.indexOf("sibilance"));
  });

  it("reports the pitch measurement without judging it", () => {
    const finding = findings({}).find((f) => f.id === "f0")!;
    expect(finding.severity).toBe("good");
    expect(finding.category).toBe("pitch");
    expect(finding.measured.unit).toBe("hz");
    expect(finding.detail.confidence).toBeGreaterThan(0);
    // A breathy take is flagged as an estimate — still not a fault.
    const breathy = findings({
      f0: { ...balancedReport().f0!, confidence: 0.3 },
    }).find((f) => f.id === "f0")!;
    expect(breathy.severity).toBe("info");
  });
});

/// <reference types="node" />
/**
 * Golden-text tests for the *Explain My Voice* wording (H-94).
 *
 * Two halves, and the second matters as much as the first:
 *
 * 1. for each synthetic report, the sentences that **must** be there — the measured number, the
 *    clause that keeps measurement apart from interpretation, the recommendation where the
 *    measurement justifies one;
 * 2. for a balanced report, the phrasing that must **never** appear — a verdict about the
 *    speaker, an absolute, an alarm. A feature that calls a healthy voice harsh once is not
 *    trusted again, so the bans are asserted, not just intended.
 */
import { readFileSync } from "fs";
import { dirname, join } from "path";
import { fileURLToPath } from "url";
import { describe, expect, it } from "vitest";

import { TONE_ZONES } from "../diagnosticsHints";
import { explainFindings, proseOf, type FindingProse } from "./prose";
import { buildVoiceSnapshot, type VoiceSnapshot } from "./snapshot";
import { IMPLAUSIBLE_NOISE_FLOOR_DBFS, MAX_SUGGESTED_TONE_GAIN_DB, NEAR_THRESHOLD_DB } from "./thresholds";
import { balancedReport, combCurve, ownerReport, type SyntheticCurve } from "./voiceFixtures";

import type { VoiceReportDto } from "../../ipc/bindings";

const repoRoot = join(dirname(fileURLToPath(import.meta.url)), "../../../../..");

/** A steady 120 Hz comb whose H2 is the loudest partial — the usual voice shape. */
function voiceCurve(): SyntheticCurve {
  return combCurve({ f0Hz: 120, harmonicsDb: [-38, -30, -36, -42, -48, -54] });
}

function snapshotOf(report: VoiceReportDto, curve: SyntheticCurve = voiceCurve()): VoiceSnapshot {
  return buildVoiceSnapshot({
    freqsHz: curve.freqsHz,
    levelsDb: curve.levelsDb,
    resolution: "bins",
    report,
    sampleRateHz: 48_000,
    origin: "average",
    nowMs: 1_700_000_000_000,
  });
}

function explain(overrides: Partial<VoiceReportDto> = {}): FindingProse[] {
  return explainFindings(snapshotOf(balancedReport(overrides)));
}

function one(prose: FindingProse[], id: string): FindingProse {
  const found = prose.find((p) => p.id === id);
  expect(found, `no prose for finding "${id}"`).toBeDefined();
  return found!;
}

/** Everything a reader would see for a finding, as one string. */
function allText(p: FindingProse): string {
  return [p.title, p.measured, p.interpretation, p.recommendation ?? ""].join(" ");
}

function wholeReport(prose: FindingProse[]): string {
  return prose.map(allText).join(" ");
}

function tone(patch: Partial<NonNullable<VoiceReportDto["tone"]>>): Partial<VoiceReportDto> {
  return { tone: { ...balancedReport().tone!, ...patch } };
}

/** The owner's own take: the real spectrum, and the report `analyze_buffer` measures from it. */
function ownerSnapshot(): VoiceSnapshot {
  const text = readFileSync(join(repoRoot, "spectrum.csv"), "utf-8");
  const rows = text.trim().split("\n").slice(1);
  const freqsHz = new Float64Array(rows.length);
  const levelsDb = new Float32Array(rows.length);
  rows.forEach((row, i) => {
    const [f = "0", db = ""] = row.split(",");
    freqsHz[i] = Number(f);
    levelsDb[i] = db === "" ? -Infinity : Number(db);
  });
  return snapshotOf(ownerReport(), { freqsHz, levelsDb });
}

describe("every finding separates measurement from interpretation", () => {
  it("gives each finding a title, a measured value and an interpretation", () => {
    const prose = explain();
    expect(prose.length).toBeGreaterThan(4);
    for (const p of prose) {
      expect(p.title.length, p.id).toBeGreaterThan(0);
      expect(p.measured.length, p.id).toBeGreaterThan(0);
      expect(p.interpretation.length, p.id).toBeGreaterThan(0);
      // The measured sentence carries a number; the interpretation is words about it.
      expect(p.measured, p.id).toMatch(/\d/);
    }
  });

  it("puts the measured number in the measured sentence, not only in the interpretation", () => {
    const prose = explain(tone({ mud_db: 10.8 }));
    expect(one(prose, "body").measured).toContain("10.8 dB");
  });

  it("names the band the number belongs to", () => {
    const prose = explain();
    expect(one(prose, "body").title).toContain("200–500 Hz");
    expect(one(prose, "presence").title).toContain("2–5 kHz");
    expect(one(prose, "air").title).toContain("10–16 kHz");
  });
});

describe("a balanced measurement is never dramatised", () => {
  const BANNED = [
    /harsh/i,
    /boomy|boominess/i,
    /muddy|muddiness/i,
    /\bbad\b|\bpoor\b|\bterrible\b|\bawful\b/i,
    /\bproblem\b|\bissue\b|\bfault\b|\bdefect\b/i,
    /\bfix\b|\bmust\b|\bshould\b/i,
    /\bexcellent\b|\bperfect\b|\bgreat\b|\bimpressive\b/i,
    /\balways\b|\bnever\b|\bcertainly\b|\bdefinitely\b/i,
    /!/,
  ];

  it("uses none of the alarmist or absolute phrasings", () => {
    const text = wholeReport(explain());
    for (const banned of BANNED) {
      expect(text, `banned phrasing ${banned} in a balanced report`).not.toMatch(banned);
    }
  });

  it("recommends nothing, because nothing crossed a threshold", () => {
    for (const p of explain()) {
      expect(p.recommendation, `${p.id} recommended something for a balanced voice`).toBeNull();
      expect(p.action, `${p.id} offered an action for a balanced voice`).toBeNull();
    }
  });

  it("never states a verdict about the speaker, in any report", () => {
    const reports: Partial<VoiceReportDto>[] = [
      {},
      tone({ mud_db: 14 }),
      tone({ presence_db: -1.3 }),
      { sibilance: { ratio_db: -8, centre_hz: 7400 } },
      { hum: { mains_hz: 50, harmonics: [1, 2, 3], strongest_hz: 50, prominence_db: 24, level_db: -52 } },
      { noise_floor_dbfs: -44, snr_db: 18, active_level_dbfs: -26 },
    ];
    for (const overrides of reports) {
      const text = wholeReport(explain(overrides));
      expect(text).not.toMatch(/your voice is/i);
      expect(text).not.toMatch(/you (must|need to|have to)/i);
    }
  });
});

describe("pitch", () => {
  it("reports the median with its note and the measured range, and judges neither", () => {
    const p = one(explain(), "f0");
    expect(p.measured).toContain("120 Hz");
    expect(p.measured).toMatch(/B2/);
    expect(p.measured).toContain("112");
    expect(p.measured).toContain("129");
    expect(p.interpretation).toMatch(/not a voice type/i);
    expect(p.recommendation).toBeNull();
  });

  it("says when the tracker needed correcting, rather than hiding it", () => {
    const report = balancedReport();
    const p = one(
      explainFindings(snapshotOf({ ...report, f0: { ...report.f0!, octave_corrected: 0.177 } })),
      "f0",
    );
    expect(p.interpretation).toContain("18%");
    expect(p.interpretation).toMatch(/octave/i);
    expect(p.interpretation).toMatch(/search floor/i);
  });

  it("says when the reading is an estimate rather than a firm measurement", () => {
    const report = balancedReport();
    const p = one(
      explainFindings(snapshotOf({ ...report, f0: { ...report.f0!, confidence: 0.4 } })),
      "f0",
    );
    expect(p.interpretation).toMatch(/estimate/i);
  });
});

describe("the strongest partial", () => {
  it("explains a spectrum whose loudest peak is not the fundamental", () => {
    const p = one(explain(), "strongest_peak");
    expect(p.measured).toMatch(/harmonic 2/i);
    expect(p.interpretation).toMatch(/does not have to be its loudest partial/i);
    expect(p.recommendation).toBeNull();
  });

  it("reconciles the two fundamentals in one clause instead of contradicting itself", () => {
    const p = one(explain(), "strongest_peak");
    // Both numbers appear, and the sentence that carries them says why they differ.
    expect(p.interpretation).toMatch(/energy average/i);
    expect(p.interpretation).toMatch(/time median/i);
    expect(p.interpretation).not.toMatch(/wrong|incorrect|disagree/i);
  });
});

describe("H-116: a peak that lines up with an unresolvable harmonic", () => {
  /**
   * The owner's second take. The pitch range (76–121 Hz, median ~106 Hz) is wide enough that
   * `highestSeparableHarmonic` = 1 — only H1 is separable — but the loudest peak in the
   * spectrum, 215.6 Hz, divided by 2 is 107.8 Hz: squarely inside the measured range. The report
   * must say the peak likely *is* H2 but cannot be separated from its neighbours this take —
   * never that it "is not a harmonic" or "a resonance of the room".
   */
  function secondTakeReport(): VoiceReportDto {
    return balancedReport({
      f0: {
        current_hz: null,
        median_hz: 106,
        low_hz: 76,
        high_hz: 121,
        voiced_fraction: 0.6,
        confidence: 0.9,
        octave_corrected: 0,
      },
    });
  }

  function secondTakeCurve(): SyntheticCurve {
    // A quiet comb (so H1/H2 aren't what findPeaks picks up) plus one loud, sharp line at
    // 215.6 Hz — the owner's reported strongest peak.
    return combCurve({
      f0Hz: 106,
      harmonicsDb: [-45, -50, -55, -60, -65, -70],
      resonances: [[215.6, -31.4, 3]],
      maxHz: 3000,
    });
  }

  it("never turns 'cannot be separated' into 'is not a harmonic — it is the room'", () => {
    const prose = explainFindings(snapshotOf(secondTakeReport(), secondTakeCurve()));
    const peak = one(prose, "strongest_peak");
    expect(peak.measured).toMatch(/215\.6 Hz/);
    expect(peak.measured).not.toMatch(/rather than a partial/i);
    expect(peak.interpretation).not.toMatch(/resonance of the (voice or of the )?room/i);
    expect(peak.interpretation).not.toMatch(/rather than a partial/i);
    // The weaker, still-true claim: it is probably H2, but the take can't be sure.
    expect(peak.interpretation).toMatch(/harmonic 2/i);
    expect(peak.interpretation).toMatch(/107\.8 Hz/);
    expect(peak.interpretation).toMatch(/pitch moved/i);
  });
});

describe("harmonics", () => {
  it("says plainly that the pitch moved too much to separate the upper harmonics", () => {
    // A ±3-semitone speaking range smears the comb from H3 up (H-91): that is a limit of the
    // measurement, and the words have to say so rather than report a weak or absent harmonic.
    const p = one(explainFindings(ownerSnapshot()), "harmonics");
    expect(p.measured).toMatch(/H\d/);
    expect(p.interpretation).toMatch(/could not be measured/i);
    expect(p.interpretation).toMatch(/pitch moved/i);
    expect(p.interpretation).toMatch(/limit of what the recording can be asked/i);
  });

  it("never implies a measurement it does not have", () => {
    const p = one(explainFindings(ownerSnapshot()), "harmonics");
    // An unresolved harmonic is not reported as weak, absent or missing.
    expect(p.interpretation).not.toMatch(/H3[^.]*\b(absent|missing|weak)\b/i);
  });

  it("says so plainly when every separable harmonic was measurable", () => {
    const p = one(explain(), "harmonics");
    expect(p.interpretation).toMatch(/every harmonic this take/i);
    expect(p.interpretation).not.toMatch(/could not be measured/i);
  });
});

describe("low-mid body", () => {
  it("states the measurement, then the possibilities — never a diagnosis", () => {
    const p = one(explain(tone({ mud_db: 10.8 })), "body");
    expect(p.measured).toContain("10.8 dB");
    expect(p.measured).toMatch(/1 kHz octave/);
    expect(p.interpretation).toMatch(/may contribute/i);
    expect(p.interpretation).toMatch(/proximity|microphone distance/i);
  });

  it("puts the source check before any EQ, and keeps the EQ move conservative", () => {
    const p = one(explain(tone({ mud_db: 10.8 })), "body");
    const rec = p.recommendation!;
    expect(rec).toBeTruthy();
    const source = rec.search(/microphone distance/i);
    const eq = rec.search(/cut/i);
    expect(source).toBeGreaterThanOrEqual(0);
    expect(eq).toBeGreaterThan(source);
    expect(p.action).toEqual({
      type: "eq",
      eq: expect.objectContaining({ kind: "cut" }),
    });
    const action = p.action!;
    const gain = action.type === "eq" ? action.eq.gainDb : 0;
    expect(Math.abs(gain)).toBeLessThanOrEqual(MAX_SUGGESTED_TONE_GAIN_DB);
  });

  it("describes a lean voice without faulting it, and suggests nothing", () => {
    const p = one(explain(tone({ mud_db: -9 })), "body");
    expect(p.interpretation).toMatch(/not required to be flat|description rather than a defect/i);
    expect(p.recommendation).toBeNull();
  });
});

describe("a finding that barely crossed its threshold", () => {
  const justOver = TONE_ZONES.presence.high + 0.7;

  it("is phrased conservatively and recommends nothing", () => {
    const p = one(explain(tone({ presence_db: justOver })), "presence");
    expect(p.nearThreshold).toBe(true);
    expect(p.interpretation).toContain("0.7 dB");
    expect(p.interpretation).toMatch(/barely across it/i);
    expect(p.interpretation).not.toMatch(/harsh|hardness/i);
    expect(p.recommendation).toBeNull();
  });

  it("is not treated like a finding that crossed by a lot", () => {
    const near = one(explain(tone({ presence_db: justOver })), "presence");
    const clear = one(explain(tone({ presence_db: TONE_ZONES.presence.high + 4 })), "presence");
    expect(clear.nearThreshold).toBe(false);
    expect(clear.interpretation).not.toEqual(near.interpretation);
    expect(clear.interpretation).toMatch(/forward/i);
  });

  it("uses the margin, not the severity alone, to decide", () => {
    const edge = TONE_ZONES.presence.high + NEAR_THRESHOLD_DB + 0.05;
    expect(one(explain(tone({ presence_db: edge })), "presence").nearThreshold).toBe(false);
  });

  it("only says hardness is worth listening for once the evidence is substantial", () => {
    const strong = one(explain(tone({ presence_db: TONE_ZONES.presence.high + 8 })), "presence");
    expect(strong.severity).toBe("significant");
    expect(strong.interpretation).toMatch(/worth listening for/i);
  });
});

describe("air", () => {
  it("treats high-frequency roll-off as normal and never advises flattening it", () => {
    const p = one(explain(tone({ air_db: -38 })), "air");
    expect(p.interpretation).toMatch(/roll-off is normal/i);
    expect(p.interpretation).toMatch(/would mostly raise hiss/i);
    expect(p.recommendation).toBeNull();
    expect(p.action).toBeNull();
  });
});

describe("sibilance", () => {
  it("suggests no de-esser while the measurement does not warrant one", () => {
    const p = one(explain({ sibilance: { ratio_db: -18, centre_hz: 6300 } }), "sibilance");
    expect(p.interpretation).toMatch(/below the point where a de-esser/i);
    expect(p.recommendation).toBeNull();
  });

  it("starts the de-esser at the detected centre, not at a default", () => {
    const p = one(explain({ sibilance: { ratio_db: -8, centre_hz: 7400 } }), "sibilance");
    expect(p.measured).toContain("7.4 kHz");
    expect(p.recommendation).toContain("7.4 kHz");
    expect(p.recommendation).not.toContain("6.3 kHz");
    expect(p.action).toEqual({ type: "copy", freqHz: 7400 });
  });
});

describe("hum", () => {
  it("says none was detected, and stops there", () => {
    const p = one(explain(), "hum");
    expect(p.measured).toMatch(/none detected/i);
    expect(p.recommendation).toBeNull();
  });

  it("points at the measured line and chases the source before EQ", () => {
    const p = one(
      explain({
        hum: { mains_hz: 50, harmonics: [1, 2, 3], strongest_hz: 150, prominence_db: 24, level_db: -52 },
      }),
      "hum",
    );
    expect(p.measured).toContain("150 Hz");
    expect(p.measured).toContain("50 Hz");
    expect(p.interpretation).toMatch(/room and the signal chain rather than to the voice/i);
    const rec = p.recommendation!;
    expect(rec.search(/source/i)).toBeLessThan(rec.search(/notch/i));
    expect(p.action).toEqual({
      type: "eq",
      eq: expect.objectContaining({ kind: "notch", freqHz: 150 }),
    });
  });
});

describe("noise", () => {
  it("labels the floor as the broadband time-domain measurement it is", () => {
    const p = one(explain(), "noise_floor");
    expect(p.measured).toMatch(/quietest 500 ms/i);
    expect(p.measured).toMatch(/time domain/i);
    expect(p.measured).not.toMatch(/FFT spectral floor estimate/i);
    expect(p.measured).toMatch(/not read off an FFT bin/i);
  });

  it("does not congratulate a floor that is probably an artefact of processing", () => {
    const prose = explainFindings(
      snapshotOf(
        balancedReport({ noise_floor_dbfs: -103, active_level_dbfs: -27.3, snr_db: 75.7 }),
      ),
    );
    const floor = one(prose, "noise_floor");
    expect(floor.interpretation).toMatch(/gated or noise-reduced/i);
    expect(floor.interpretation).toMatch(/describes the file/i);
    expect(floor.interpretation).not.toMatch(/excellent|outstanding|great|well done/i);

    const snr = one(prose, "snr");
    expect(snr.interpretation).toMatch(/property of the file/i);
    expect(snr.interpretation).not.toMatch(/clean separation/i);
  });

  it("still calls a plausible quiet floor what it is", () => {
    const prose = explain({ noise_floor_dbfs: IMPLAUSIBLE_NOISE_FLOOR_DBFS + 5 });
    expect(one(prose, "noise_floor").interpretation).not.toMatch(/gated/i);
    expect(one(prose, "snr").interpretation).toMatch(/clean separation/i);
  });

  it("puts level and distance before noise reduction when the floor is high", () => {
    const prose = explain({ noise_floor_dbfs: -44, active_level_dbfs: -26, snr_db: 18 });
    expect(one(prose, "noise_floor").recommendation).toMatch(/before noise reduction/i);
    expect(one(prose, "snr").recommendation).toMatch(/microphone/i);
  });

  it("suggests a high-pass only when rumble is actually elevated", () => {
    expect(one(explain(), "rumble").recommendation).toBeNull();
    const loud = one(explain({ rumble_db: -18 }), "rumble");
    expect(loud.interpretation).toMatch(/traffic, ventilation, footfall or handling/i);
    expect(loud.action).toEqual({
      type: "eq",
      eq: expect.objectContaining({ kind: "high_pass", freqHz: 80 }),
    });
  });
});

describe("a report with almost nothing measurable", () => {
  it("invents no prose for a measurement that was not taken", () => {
    const prose = explainFindings(
      snapshotOf({
        f0: null,
        tone: null,
        sibilance: null,
        hum: null,
        rumble_db: null,
        noise_floor_dbfs: null,
        active_level_dbfs: null,
        snr_db: null,
        span_s: 1.2,
      }),
    );
    expect(prose).toEqual([]);
  });
});

describe("the owner's own recording", () => {
  it("reads the loudest peak as H2 and explains the two fundamentals", () => {
    const prose = explainFindings(ownerSnapshot());
    const peak = one(prose, "strongest_peak");
    expect(peak.measured).toMatch(/198\.\d Hz/);
    expect(peak.measured).toMatch(/harmonic 2/i);
    expect(peak.interpretation).toMatch(/energy average/i);
    expect(peak.interpretation).toMatch(/time median/i);
    expect(peak.interpretation).toMatch(/99\.\d Hz/);
    expect(peak.interpretation).toContain("103.6 Hz");
  });

  it("calls the low-mid energy elevated without calling the voice boomy", () => {
    const body = one(explainFindings(ownerSnapshot()), "body");
    expect(body.severity).toBe("attention");
    expect(body.measured).toContain("10.3 dB");
    expect(body.interpretation).toMatch(/may contribute to warmth or to boominess/i);
    expect(body.recommendation).toMatch(/microphone distance/i);
  });

  it("is conservative about a presence reading that crossed by 0.7 dB", () => {
    const presence = one(explainFindings(ownerSnapshot()), "presence");
    expect(presence.severity).toBe("attention");
    expect(presence.nearThreshold).toBe(true);
    expect(presence.interpretation).toContain("0.7 dB");
    expect(presence.interpretation).not.toMatch(/harsh|hardness/i);
    expect(presence.recommendation).toBeNull();
  });

  it("does not report hum that is not there", () => {
    expect(one(explainFindings(ownerSnapshot()), "hum").measured).toMatch(/none detected/i);
  });

  it("treats the −103 dBFS floor as suspicious rather than as an achievement", () => {
    const prose = explainFindings(ownerSnapshot());
    expect(one(prose, "noise_floor").interpretation).toMatch(/gated or noise-reduced/i);
    expect(one(prose, "snr").interpretation).toMatch(/property of the file/i);
  });

  it("says which harmonics could not be separated and why", () => {
    const harmonics = one(explainFindings(ownerSnapshot()), "harmonics");
    expect(harmonics.interpretation).toMatch(/H3 upwards could not be measured/i);
    expect(harmonics.interpretation).toMatch(/pitch moved/i);
  });
});

describe("proseOf", () => {
  it("explains a single finding the same way the whole report does", () => {
    const snapshot = snapshotOf(balancedReport());
    const finding = snapshot.findings.find((f) => f.id === "body")!;
    expect(proseOf(finding, snapshot)).toEqual(one(explainFindings(snapshot), "body"));
  });
});

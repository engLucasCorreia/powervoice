/// <reference types="node" />
/**
 * Golden-text tests for the engineering summary (H-94 §2–§3): the voice profile is composed
 * from the real measurements, the suggested focus is composed from the findings that actually
 * crossed a threshold, and a clean recording is told it is clean rather than given a finding per
 * section so the panel looks busy.
 */
import { readFileSync } from "fs";
import { dirname, join } from "path";
import { fileURLToPath } from "url";
import { describe, expect, it } from "vitest";

import { TONE_ZONES } from "../diagnosticsHints";
import { buildVoiceSnapshot, type VoiceSnapshot } from "./snapshot";
import { buildVoiceSummary, suggestedEqBands } from "./summary";
import { MAX_SUGGESTED_TONE_GAIN_DB } from "./thresholds";
import { balancedReport, combCurve, ownerReport, type SyntheticCurve } from "./voiceFixtures";

import type { VoiceReportDto } from "../../ipc/bindings";

const repoRoot = join(dirname(fileURLToPath(import.meta.url)), "../../../../..");

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

function summaryOf(overrides: Partial<VoiceReportDto> = {}) {
  return buildVoiceSummary(snapshotOf(balancedReport(overrides)));
}

function tone(patch: Partial<NonNullable<VoiceReportDto["tone"]>>): Partial<VoiceReportDto> {
  return { tone: { ...balancedReport().tone!, ...patch } };
}

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

describe("the voice profile", () => {
  it("is the six readings the ticket asks for, each carrying its measurement", () => {
    const summary = summaryOf();
    expect(summary.profile.map((p) => p.id)).toEqual([
      "f0",
      "body",
      "presence",
      "sibilance",
      "rumble",
      "hum",
    ]);
    for (const entry of summary.profile) {
      expect(entry.label.length, entry.id).toBeGreaterThan(0);
      expect(entry.value.length, entry.id).toBeGreaterThan(0);
      expect(entry.reading.length, entry.id).toBeGreaterThan(0);
    }
  });

  it("shows the median pitch with its note", () => {
    const pitch = summaryOf().profile[0]!;
    expect(pitch.value).toContain("120 Hz");
    expect(pitch.value).toMatch(/B2/);
    expect(pitch.reading).toMatch(/not a voice type/i);
  });

  it("carries the real numbers, not a restatement of the severity", () => {
    const summary = summaryOf(tone({ mud_db: 10.8 }));
    expect(summary.profile.find((p) => p.id === "body")!.value).toContain("10.8 dB");
    expect(summary.profile.find((p) => p.id === "sibilance")!.value).toContain("6.3 kHz");
  });

  it("says a measurement was not taken rather than inventing one", () => {
    const summary = summaryOf({ sibilance: null, rumble_db: null });
    expect(summary.profile.find((p) => p.id === "sibilance")!.value).toMatch(/not measured/i);
    expect(summary.profile.find((p) => p.id === "rumble")!.reading).toMatch(/not measured/i);
  });

  it("distinguishes a reading that barely crossed from one that crossed clearly", () => {
    const near = summaryOf(tone({ presence_db: TONE_ZONES.presence.high + 0.7 }));
    const clear = summaryOf(tone({ presence_db: TONE_ZONES.presence.high + 5 }));
    expect(near.profile.find((p) => p.id === "presence")!.reading).toMatch(/just above/i);
    expect(clear.profile.find((p) => p.id === "presence")!.reading).not.toMatch(/just above/i);
  });
});

describe("the headline", () => {
  it("says so when nothing crossed a threshold", () => {
    const summary = summaryOf();
    expect(summary.headline).toMatch(/nothing measured in this take crossed/i);
    expect(summary.headline).toMatch(/not a list of problems/i);
  });

  it("names what crossed, and says the rest did not", () => {
    const summary = summaryOf(tone({ mud_db: 10.8 }));
    expect(summary.headline).toMatch(/one measurement crossed/i);
    expect(summary.headline).toContain("Low-mid body");
    expect(summary.headline).toMatch(/everything else sat inside its usual range/i);
  });

  it("counts correctly when more than one crossed", () => {
    const summary = summaryOf({
      ...tone({ mud_db: 10.8 }),
      rumble_db: -18,
    });
    expect(summary.headline).toMatch(/^2 measurements crossed/i);
  });
});

describe("the suggested focus", () => {
  it("is empty of advice for a clean recording, and says so once", () => {
    const summary = summaryOf();
    expect(summary.focus).toHaveLength(1);
    expect(summary.focus[0]!.text).toMatch(/no action suggested/i);
    expect(summary.focus[0]!.action).toBeNull();
  });

  it("puts the source check before the EQ move", () => {
    const summary = summaryOf(tone({ mud_db: 10.8 }));
    expect(summary.focus[0]!.text).toMatch(/microphone distance/i);
    expect(summary.focus[0]!.text.search(/microphone distance/i)).toBeLessThan(
      summary.focus[0]!.text.search(/EQ/),
    );
  });

  it("orders the worst finding first", () => {
    const summary = summaryOf({
      ...tone({ mud_db: 10.8 }),
      hum: { mains_hz: 50, harmonics: [1, 2], strongest_hz: 50, prominence_db: 26, level_db: -50 },
    });
    expect(summary.focus[0]!.id).toBe("hum");
  });

  it("flags a floor that looks processed even though nothing crossed a threshold", () => {
    const summary = summaryOf({ noise_floor_dbfs: -103, active_level_dbfs: -27.3, snr_db: 75.7 });
    const processed = summary.focus.find((f) => f.id === "noise_processing");
    expect(processed?.text).toMatch(/gated or noise-reduced/i);
    expect(processed?.action).toBeNull();
    // It is an observation, not an action — the headline still says nothing crossed.
    expect(summary.headline).toMatch(/nothing measured in this take crossed/i);
  });

  it("de-esses at the measured centre", () => {
    const summary = summaryOf({ sibilance: { ratio_db: -8, centre_hz: 7400 } });
    const item = summary.focus.find((f) => f.id === "sibilance");
    expect(item?.text).toContain("7.4 kHz");
  });
});

describe("the EQ suggestion overlay", () => {
  it("is empty when nothing justifies a move", () => {
    expect(suggestedEqBands(snapshotOf(balancedReport()))).toEqual([]);
  });

  it("offers nothing for a finding that barely crossed", () => {
    const snapshot = snapshotOf(balancedReport(tone({ presence_db: TONE_ZONES.presence.high + 0.7 })));
    expect(suggestedEqBands(snapshot)).toEqual([]);
  });

  it("keeps every tone move within ±3 dB", () => {
    const snapshot = snapshotOf(
      balancedReport({ ...tone({ mud_db: 16, presence_db: -20 }), rumble_db: -10 }),
    );
    for (const band of suggestedEqBands(snapshot)) {
      if (band.kind === "cut" || band.kind === "boost") {
        expect(Math.abs(band.gainDb), `${band.kind} at ${band.freqHz} Hz`).toBeLessThanOrEqual(
          MAX_SUGGESTED_TONE_GAIN_DB,
        );
      }
    }
  });

  it("uses the shapes the existing 'Add EQ band here' path already applies", () => {
    const snapshot = snapshotOf(balancedReport(tone({ mud_db: 16 })));
    const bands = suggestedEqBands(snapshot);
    expect(bands).toHaveLength(1);
    expect(bands[0]).toEqual({ kind: "cut", freqHz: 300, gainDb: -3, q: 1.4 });
  });

  it("notches hum at the measured line, which is corrective rather than tone shaping", () => {
    const snapshot = snapshotOf(
      balancedReport({
        hum: { mains_hz: 50, harmonics: [1], strongest_hz: 50, prominence_db: 26, level_db: -50 },
      }),
    );
    expect(suggestedEqBands(snapshot)[0]).toEqual(
      expect.objectContaining({ kind: "notch", freqHz: 50 }),
    );
  });
});

describe("the owner's own recording", () => {
  it("leads with the low-mid body and keeps the presence reading in its place", () => {
    const summary = buildVoiceSummary(ownerSnapshot());
    expect(summary.headline).toMatch(/2 measurements crossed/i);
    expect(summary.headline).toContain("Low-mid body");
    expect(summary.focus[0]!.id).toBe("body");
    const presence = summary.focus.find((f) => f.id === "presence");
    expect(presence?.text).toMatch(/close enough to the line to leave alone/i);
    expect(presence?.action).toBeNull();
  });

  it("names the processed-looking floor as an observation", () => {
    const summary = buildVoiceSummary(ownerSnapshot());
    expect(summary.focus.some((f) => f.id === "noise_processing")).toBe(true);
  });

  it("profiles the voice from the measurements", () => {
    const summary = buildVoiceSummary(ownerSnapshot());
    const by = (id: string) => summary.profile.find((p) => p.id === id)!;
    expect(by("f0").value).toContain("104 Hz");
    expect(by("body").value).toContain("10.3 dB");
    expect(by("presence").value).toContain("1.3 dB");
    expect(by("sibilance").value).toContain("4.8 kHz");
    expect(by("hum").reading).toMatch(/none detected/i);
    expect(summary.basis).toContain("31.5");
  });

  it("suggests exactly one EQ move — the conservative low-mid cut", () => {
    expect(suggestedEqBands(ownerSnapshot())).toEqual([
      { kind: "cut", freqHz: 300, gainDb: -3, q: 1.4 },
    ]);
  });
});

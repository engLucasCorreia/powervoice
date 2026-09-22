import { describe, expect, it } from "vitest";
import { buildExplainExportData, explainExportDateLabel, explainExportFileBase } from "./explainExport";
import type { ExplainGraphExportFrame } from "./explainExport";
import { explainFindings } from "./prose";
import { buildVoiceSnapshot, type VoiceSnapshot } from "./snapshot";
import { buildVoiceSummary } from "./summary";
import { balancedReport, combCurve, ownerReport } from "./voiceFixtures";

/**
 * H-115 ticket §4: the export renders exactly what is already on screen — nothing here re-measures
 * or reclassifies a finding, so these tests are mostly "the data survives the trip unchanged".
 */

function snapshotFrom(reportOverrides: Parameters<typeof balancedReport>[0] = {}): VoiceSnapshot {
  const curve = combCurve({ f0Hz: 120, harmonicsDb: [-38, -30, -36, -42, -48, -54] });
  return buildVoiceSnapshot({
    freqsHz: curve.freqsHz,
    levelsDb: curve.levelsDb,
    resolution: "bins",
    report: balancedReport(reportOverrides),
    sampleRateHz: 48_000,
    origin: "average",
    nowMs: 1_726_000_000_000,
  });
}

function emptyGraphFrame(): ExplainGraphExportFrame {
  return {
    canvas: document.createElement("canvas"),
    widthPx: 800,
    heightPx: 320,
    plot: { x: 48, y: 10, width: 742, height: 290 },
    cards: [],
    leaders: [],
  };
}

describe("explainExportDateLabel", () => {
  it("formats the snapshot's timestamp for a reader", () => {
    const label = explainExportDateLabel(1_726_000_000_000);
    expect(label.length).toBeGreaterThan(0);
    // A locale-formatted date always carries a 4-digit year somewhere in it.
    expect(label).toMatch(/\d{4}/);
  });
});

describe("explainExportFileBase", () => {
  it("is stable for the same snapshot moment, so an image and its report share a base name", () => {
    const a = explainExportFileBase({ takenAtMs: 1_726_000_000_000 });
    const b = explainExportFileBase({ takenAtMs: 1_726_000_000_000 });
    expect(a).toBe(b);
  });

  it("carries the analysis date (YYYY-MM-DD) in the name", () => {
    const base = explainExportFileBase({ takenAtMs: Date.UTC(2026, 8, 21, 12, 0, 0) });
    expect(base).toBe("voice-spectrum-analysis-2026-09-21");
  });
});

describe("buildExplainExportData", () => {
  it("carries the take's duration and the analysis date, as the ticket requires", () => {
    const snapshot = snapshotFrom({ span_s: 8.4 });
    const summary = buildVoiceSummary(snapshot);
    const prose = explainFindings(snapshot);
    const data = buildExplainExportData({
      snapshot,
      summary,
      prose,
      subtitle: "8.4 s of audio analysed",
      graph: emptyGraphFrame(),
      showAnnotations: true,
      showEqAdvice: true,
      beneathFindings: [],
    });

    expect(data.durationLabel).toContain("8.4");
    expect(data.takenAtMs).toBe(snapshot.takenAtMs);
    expect(data.dateLabel.length).toBeGreaterThan(0);
  });

  it("carries every finding's prose, not only the ones the graph had room for", () => {
    const snapshot = snapshotFrom();
    const summary = buildVoiceSummary(snapshot);
    const prose = explainFindings(snapshot);
    const data = buildExplainExportData({
      snapshot,
      summary,
      prose,
      subtitle: "",
      graph: emptyGraphFrame(),
      showAnnotations: true,
      showEqAdvice: true,
      beneathFindings: [],
    });

    expect(data.findings).toHaveLength(prose.length);
    expect(data.findings.map((p) => p.id)).toEqual(prose.map((p) => p.id));
  });

  it("maps beneathFindings to their matching prose for the 'also measured' cards", () => {
    const snapshot = snapshotFrom();
    const summary = buildVoiceSummary(snapshot);
    const prose = explainFindings(snapshot);
    const f0Finding = snapshot.findings.find((f) => f.id === "f0")!;
    const data = buildExplainExportData({
      snapshot,
      summary,
      prose,
      subtitle: "",
      graph: emptyGraphFrame(),
      showAnnotations: true,
      showEqAdvice: true,
      beneathFindings: [f0Finding],
    });

    expect(data.beneath).toHaveLength(1);
    expect(data.beneath[0]!.title).toBe(prose.find((p) => p.id === "f0")!.title);
  });

  it("carries the current toggle state (annotations/EQ advice) through unchanged", () => {
    const snapshot = snapshotFrom();
    const summary = buildVoiceSummary(snapshot);
    const prose = explainFindings(snapshot);
    const data = buildExplainExportData({
      snapshot,
      summary,
      prose,
      subtitle: "",
      graph: emptyGraphFrame(),
      showAnnotations: false,
      showEqAdvice: false,
      beneathFindings: [],
    });

    expect(data.showAnnotations).toBe(false);
    expect(data.showEqAdvice).toBe(false);
  });

  it("passes the graph frame through untouched (the same canvas reference)", () => {
    const snapshot = snapshotFrom();
    const summary = buildVoiceSummary(snapshot);
    const prose = explainFindings(snapshot);
    const graph = emptyGraphFrame();
    const data = buildExplainExportData({
      snapshot,
      summary,
      prose,
      subtitle: "",
      graph,
      showAnnotations: true,
      showEqAdvice: true,
      beneathFindings: [],
    });

    expect(data.graph).toBe(graph);
  });

  it("reflects the owner's real near-threshold take (H-115's motivating example)", () => {
    const snapshot = buildVoiceSnapshot({
      freqsHz: combCurve({ f0Hz: 103, harmonicsDb: [-40, -34, -38] }).freqsHz,
      levelsDb: combCurve({ f0Hz: 103, harmonicsDb: [-40, -34, -38] }).levelsDb,
      resolution: "bins",
      report: ownerReport(),
      sampleRateHz: 48_000,
      origin: "average",
    });
    const summary = buildVoiceSummary(snapshot);
    const prose = explainFindings(snapshot);
    const data = buildExplainExportData({
      snapshot,
      summary,
      prose,
      subtitle: "",
      graph: emptyGraphFrame(),
      showAnnotations: true,
      showEqAdvice: true,
      beneathFindings: [],
    });

    // The report table must still carry body's measured value and its interpretation, whatever
    // the graph itself did with it.
    const body = data.findings.find((p) => p.id === "body");
    expect(body).toBeDefined();
    expect(body!.measured.length).toBeGreaterThan(0);
    expect(body!.interpretation.length).toBeGreaterThan(0);
  });
});

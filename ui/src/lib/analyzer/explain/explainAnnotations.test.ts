import { describe, expect, it } from "vitest";
import type { VoiceFinding } from "./findings";
import {
  anchorFreqHz,
  buildAnnotationItem,
  compactCardHeightPx,
  compactCardWidthPx,
  layoutExplainAnnotations,
  wrapLines,
} from "./explainAnnotations";
import { balancedReport, combCurve } from "./voiceFixtures";
import { buildVoiceSnapshot, type VoiceSnapshot } from "./snapshot";
import { explainFindings, proseOf } from "./prose";

const IDENTITY_GEOMETRY = { xForFreq: (f: number) => f, yForCurveAtFreq: (f: number) => 1000 - f };

function snapshotFor(f0Hz = 120): VoiceSnapshot {
  const curve = combCurve({ f0Hz, harmonicsDb: [-38, -30, -36, -42, -48, -54] });
  return buildVoiceSnapshot({
    freqsHz: curve.freqsHz,
    levelsDb: curve.levelsDb,
    resolution: "bins",
    report: balancedReport({ f0: { ...balancedReport().f0!, median_hz: f0Hz, low_hz: f0Hz * 0.95, high_hz: f0Hz * 1.05 } }),
    sampleRateHz: 48_000,
    origin: "average",
  });
}

describe("anchorFreqHz (H-92/H-93 seam)", () => {
  it("a frequency anchor is itself", () => {
    expect(anchorFreqHz({ anchor: { kind: "frequency", freqHz: 250 } } as VoiceFinding)).toBe(250);
  });

  it("a band anchor is its geometric mid, the log-axis centre of the measured band", () => {
    expect(anchorFreqHz({ anchor: { kind: "band", lowHz: 200, highHz: 500 } } as VoiceFinding)).toBeCloseTo(
      Math.sqrt(200 * 500),
      6,
    );
  });

  it("no anchor means no frequency", () => {
    expect(anchorFreqHz({ anchor: { kind: "none" } } as VoiceFinding)).toBeNull();
  });
});

describe("buildAnnotationItem", () => {
  it("places the card's anchor at the real measured point, in plot pixels, using H-94's words", () => {
    const snapshot = snapshotFor();
    const finding = snapshot.findings.find((f) => f.id === "f0")!;
    const prose = proseOf(finding, snapshot);
    const item = buildAnnotationItem(finding, prose, IDENTITY_GEOMETRY);
    expect(item).not.toBeNull();
    expect(item!.anchor).toEqual({ x: 120, y: 880 });
    // H-93 sorts ascending; H-91's priority is highest-first, so the seam negates it.
    expect(item!.priority).toBe(-finding.priority);
    expect(item!.prose.title).toBeTruthy();
    expect(item!.prose.measured).toBeTruthy();
  });

  it("returns null for a finding with nothing to point at", () => {
    const snapshot = snapshotFor();
    const finding = snapshot.findings.find((f) => f.id === "snr")!;
    const prose = proseOf(finding, snapshot);
    expect(buildAnnotationItem(finding, prose, IDENTITY_GEOMETRY)).toBeNull();
  });
});

describe("layoutExplainAnnotations", () => {
  it("sends every real finding through, split between placed and beneath", () => {
    const snapshot = snapshotFor();
    const findings = snapshot.findings;
    const prose = explainFindings(snapshot);
    const { placed, beneath } = layoutExplainAnnotations(findings, prose, IDENTITY_GEOMETRY, {
      rect: { x: 0, y: 0, width: 2000, height: 1000 },
    });
    const total = placed.length + beneath.length;
    expect(total).toBe(findings.length);
    // snr/noise_floor/hum(absent) never anchor to a frequency — always "beneath".
    expect(beneath.some((f) => f.id === "snr")).toBe(true);
  });

  it("caps the graph at maxLabels (mobile: top three) and moves the rest beneath", () => {
    const snapshot = snapshotFor();
    const findings = snapshot.findings;
    const prose = explainFindings(snapshot);
    const anchored = findings.filter((f) => f.anchor.kind !== "none");
    expect(anchored.length).toBeGreaterThan(3);
    const { placed, beneath } = layoutExplainAnnotations(findings, prose, IDENTITY_GEOMETRY, {
      rect: { x: 0, y: 0, width: 2000, height: 1000 },
      maxLabels: 3,
    });
    expect(placed.length).toBeLessThanOrEqual(3);
    expect(beneath.length).toBe(findings.length - placed.length);
    // The highest-priority anchored findings are the ones kept on the graph.
    const keptIds = new Set(placed.map((p) => p.item.finding.id));
    const topByPriority = [...anchored].sort((a, b) => b.priority - a.priority).slice(0, placed.length);
    for (const f of topByPriority) {
      expect(keptIds.has(f.id)).toBe(true);
    }
  });

  it("beneath findings come back highest H-91 priority first", () => {
    const snapshot = snapshotFor();
    const findings = snapshot.findings;
    const prose = explainFindings(snapshot);
    const { beneath } = layoutExplainAnnotations(findings, prose, IDENTITY_GEOMETRY, {
      rect: { x: 0, y: 0, width: 10, height: 10 }, // tiny: almost everything is dropped
      maxLabels: 0,
    });
    for (let i = 1; i < beneath.length; i++) {
      expect(beneath[i - 1]!.priority).toBeGreaterThanOrEqual(beneath[i]!.priority);
    }
  });

  it("never invents an anchor: a card's leader line ends where the geometry says the curve is", () => {
    const snapshot = snapshotFor();
    const findings = snapshot.findings;
    const prose = explainFindings(snapshot);
    const { placed } = layoutExplainAnnotations(findings, prose, IDENTITY_GEOMETRY, {
      rect: { x: 0, y: 0, width: 2000, height: 1000 },
    });
    for (const p of placed) {
      const freq = anchorFreqHz(p.item.finding)!;
      expect(p.leader.to).toEqual({ x: IDENTITY_GEOMETRY.xForFreq(freq), y: IDENTITY_GEOMETRY.yForCurveAtFreq(freq) });
    }
  });

  it("the compact card's width scales with the plot it sits on (H-102: no more fixed guess)", () => {
    const snapshot = snapshotFor();
    const findings = snapshot.findings;
    const prose = explainFindings(snapshot);
    const wide = layoutExplainAnnotations(findings, prose, IDENTITY_GEOMETRY, {
      rect: { x: 0, y: 0, width: 2000, height: 1000 },
    });
    const narrow = layoutExplainAnnotations(findings, prose, IDENTITY_GEOMETRY, {
      rect: { x: 0, y: 0, width: 320, height: 1000 },
    });
    expect(wide.placed[0]!.item.width).toBeGreaterThan(narrow.placed[0]!.item.width);
  });
});

// H-102: the graph's floating cards used to truncate H-94's full measured sentences mid-word
// ("The loudest peak in the spectrum is th…") because their box was a fixed 34 px tall with a
// single `nowrap` line. They must now wrap to fit their real content instead.
describe("wrapLines", () => {
  it("reconstructs the original words — nothing is dropped or truncated", () => {
    const text = "The loudest peak in the spectrum is at 199.2 Hz, -31.3 dBFS.";
    const lines = wrapLines(text, 120, 11);
    expect(lines.length).toBeGreaterThan(1);
    expect(lines.join(" ")).toBe(text);
  });

  it("fits everything on one line when the box is wide enough", () => {
    const text = "Median 130 Hz";
    expect(wrapLines(text, 400, 11)).toEqual([text]);
  });

  it("gives an unbreakably long single word its own line rather than looping forever", () => {
    const text = "a".repeat(80);
    const lines = wrapLines(text, 40, 11);
    expect(lines).toEqual([text]);
  });

  it("is empty for empty input", () => {
    expect(wrapLines("", 200, 11)).toEqual([]);
  });
});

describe("compactCardHeightPx (H-102: a real box for the real sentence, never a fixed guess)", () => {
  it("grows with a longer measured sentence at a fixed width", () => {
    const short = compactCardHeightPx("Median 130 Hz", 200);
    const long = compactCardHeightPx(
      "The loudest peak in the spectrum is at 199.2 Hz, -31.3 dBFS. It does not line up with any harmonic.",
      200,
    );
    expect(long).toBeGreaterThan(short);
  });

  it("a narrower box needs more height for the same sentence", () => {
    const text = "Median 130 Hz (C3 -16 cents) over the voiced frames of this take, a spread of 3.2 semitones.";
    const narrow = compactCardHeightPx(text, 140);
    const wide = compactCardHeightPx(text, 260);
    expect(narrow).toBeGreaterThanOrEqual(wide);
  });
});

describe("compactCardWidthPx", () => {
  it("clamps to a legible minimum on a very narrow (phone) plot", () => {
    expect(compactCardWidthPx(200)).toBeGreaterThanOrEqual(120);
  });

  it("clamps to a non-dominant maximum on a very wide plot", () => {
    const width = compactCardWidthPx(4000);
    expect(width).toBeLessThan(4000 * 0.34);
  });

  it("scales roughly linearly between the two clamps", () => {
    expect(compactCardWidthPx(600)).toBeGreaterThan(compactCardWidthPx(300));
  });
});

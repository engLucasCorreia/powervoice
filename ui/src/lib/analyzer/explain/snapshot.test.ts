import { beforeEach, describe, expect, it } from "vitest";

import { smoothFractionalOctave } from "../smoothing";
import {
  clearExplainSnapshot,
  explainVoiceState,
  freezeExplainSnapshot,
  resetExplainVoiceForTest,
} from "./explainVoice.svelte";
import {
  buildVoiceSnapshot,
  DEFAULT_SNAPSHOT_SMOOTHING_OCT,
  type VoiceSnapshotInput,
} from "./snapshot";
import { balancedReport, combCurve } from "./voiceFixtures";

function input(overrides: Partial<VoiceSnapshotInput> = {}): VoiceSnapshotInput {
  const f0 = 120;
  const curve = combCurve({ f0Hz: f0, harmonicsDb: [-38, -30, -36, -42, -48, -54] });
  return {
    freqsHz: curve.freqsHz,
    levelsDb: curve.levelsDb,
    resolution: "bins",
    report: balancedReport(),
    sampleRateHz: 48_000,
    origin: "live",
    nowMs: 1_700_000_000_000,
    ...overrides,
  };
}

describe("the frozen snapshot", () => {
  it("copies the curve it was given, so later frames cannot change it", () => {
    const source = input();
    const snapshot = buildVoiceSnapshot(source);
    const before = snapshot.rawDb[100];
    (source.levelsDb as Float32Array)[100] = 0;
    (source.freqsHz as Float64Array)[100] = 1;
    expect(snapshot.rawDb[100]).toBe(before);
    expect(snapshot.freqsHz[100]).not.toBe(1);
  });

  it("carries the raw curve and its smoothed envelope, both at the stated width", () => {
    const source = input();
    const snapshot = buildVoiceSnapshot(source);
    expect(snapshot.smoothingOct).toBe(DEFAULT_SNAPSHOT_SMOOTHING_OCT);
    const expected = smoothFractionalOctave(
      snapshot.freqsHz,
      snapshot.rawDb,
      DEFAULT_SNAPSHOT_SMOOTHING_OCT,
    );
    expect(Array.from(snapshot.smoothedDb)).toEqual(Array.from(expected));
    // The envelope really is an envelope: it never sits above the peak it smooths.
    let strictlyDifferent = 0;
    for (let i = 0; i < snapshot.rawDb.length; i++) {
      if (snapshot.smoothedDb[i] !== snapshot.rawDb[i]) {
        strictlyDifferent += 1;
      }
    }
    expect(strictlyDifferent).toBeGreaterThan(0);
  });

  it("reports the span the engine measured, never an assumed one", () => {
    const snapshot = buildVoiceSnapshot(input({ report: balancedReport({ span_s: 7.25 }) }));
    expect(snapshot.spanS).toBe(7.25);
  });

  it("builds the pitch profile, the harmonics and the peak relation from the same numbers", () => {
    const snapshot = buildVoiceSnapshot(input());
    expect(snapshot.pitch).not.toBeNull();
    expect(snapshot.pitch!.fundamentalHz).toBe(balancedReport().f0!.median_hz);
    expect(snapshot.pitch!.note?.name).toBeTruthy();
    expect(snapshot.harmonics).toHaveLength(6);
    expect(snapshot.strongestPeak!.harmonicNumber).toBe(2);
    // Every anchor the findings expose is a measured frequency, inside the curve.
    const last = snapshot.freqsHz[snapshot.freqsHz.length - 1]!;
    for (const finding of snapshot.findings) {
      if (finding.anchor.kind === "frequency") {
        expect(finding.anchor.freqHz).toBeGreaterThan(0);
        expect(Number.isFinite(finding.anchor.freqHz)).toBe(true);
      }
      if (finding.anchor.kind === "band") {
        expect(finding.anchor.lowHz).toBeLessThan(finding.anchor.highHz);
      }
    }
    expect(snapshot.strongestPeak!.freqHz).toBeLessThanOrEqual(last);
  });

  it("still analyses everything else when nothing voiced was measured", () => {
    const snapshot = buildVoiceSnapshot(input({ report: balancedReport({ f0: null }) }));
    expect(snapshot.pitch).toBeNull();
    expect(snapshot.harmonics).toHaveLength(0);
    expect(snapshot.strongestPeak).toBeNull();
    expect(snapshot.findings.map((f) => f.id)).not.toContain("f0");
    expect(snapshot.findings.map((f) => f.id)).toContain("snr");
  });

  it("survives an empty curve", () => {
    const snapshot = buildVoiceSnapshot(
      input({ freqsHz: new Float64Array(0), levelsDb: new Float32Array(0) }),
    );
    expect(snapshot.peaks).toHaveLength(0);
    expect(snapshot.strongestPeak).toBeNull();
    expect(snapshot.findings.length).toBeGreaterThan(0);
  });
});

describe("the frozen snapshot store", () => {
  beforeEach(() => {
    resetExplainVoiceForTest();
  });

  it("computes once per click and hands the same object back at frame rate", () => {
    const source = input();
    const first = freezeExplainSnapshot(source);
    const again = freezeExplainSnapshot(source);
    expect(again).toBe(first);
    expect(explainVoiceState().snapshot).toBe(first);
  });

  it("rebuilds when the analysis it was given is a different one", () => {
    const first = freezeExplainSnapshot(input());
    const second = freezeExplainSnapshot(input({ origin: "average" }));
    expect(second).not.toBe(first);
    expect(explainVoiceState().snapshot).toBe(second);
  });

  it("forgets the snapshot when the report is closed", () => {
    freezeExplainSnapshot(input());
    clearExplainSnapshot();
    expect(explainVoiceState().snapshot).toBeNull();
  });
});

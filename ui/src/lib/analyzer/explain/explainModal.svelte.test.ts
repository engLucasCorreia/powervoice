import { afterEach, describe, expect, it } from "vitest";
import { explainVoiceState } from "./explainVoice.svelte";
import { closeExplainVoice, explainModalState, openExplainVoice, resetExplainModalForTest } from "./explainModal.svelte";
import { balancedReport, combCurve } from "./voiceFixtures";
import type { VoiceSnapshotInput } from "./snapshot";

function input(overrides: Partial<VoiceSnapshotInput> = {}): VoiceSnapshotInput {
  const curve = combCurve({ f0Hz: 120, harmonicsDb: [-38, -30, -36] });
  return {
    freqsHz: curve.freqsHz,
    levelsDb: curve.levelsDb,
    resolution: "bins",
    report: balancedReport(),
    sampleRateHz: 48_000,
    origin: "average",
    ...overrides,
  };
}

afterEach(() => {
  resetExplainModalForTest();
});

describe("the Explain My Voice modal store (H-92)", () => {
  it("starts closed, with nothing frozen", () => {
    expect(explainModalState().open).toBe(false);
    expect(explainVoiceState().snapshot).toBeNull();
  });

  it("opening freezes the given analysis and shows the modal", () => {
    openExplainVoice(input());
    expect(explainModalState().open).toBe(true);
    expect(explainVoiceState().snapshot).not.toBeNull();
    expect(explainVoiceState().snapshot!.origin).toBe("average");
  });

  it("closing hides the modal and drops the frozen analysis, so live analysis is untouched", () => {
    openExplainVoice(input());
    closeExplainVoice();
    expect(explainModalState().open).toBe(false);
    expect(explainVoiceState().snapshot).toBeNull();
  });
});

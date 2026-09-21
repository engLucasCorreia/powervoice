import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { afterEach, describe, expect, it } from "vitest";
import type { RackSlotDto, RackStateDto } from "../ipc/bindings";
import { rackSlotDto, rackStateDto } from "../test/fixtures";
import { resetRackForTest } from "../rack/rack.svelte";
import { applyEqAction } from "./eqApply";
import { EQ_IDS, EQ_MODULE_ID, eqSlotIndex, planEqAction, previewEqOverrides } from "./eqSuggest";

/** A Parametric EQ slot at SPEC-015 defaults (peak bands 200/500/1200/3000/6000 Hz, 0 dB). */
function eqSlot(gains: number[] = [0, 0, 0, 0, 0]): RackSlotDto {
  const freqs = [200, 500, 1200, 3000, 6000];
  const values = EQ_IDS.peaks.flatMap((ids, i) => [
    { id: ids.on, value: 1, normalized: 1, text: "On" },
    { id: ids.freq, value: freqs[i]!, normalized: 0, text: "" },
    { id: ids.gain, value: gains[i]!, normalized: 0.5, text: "" },
    { id: ids.q, value: 1, normalized: 0.5, text: "" },
  ]);
  values.push({ id: EQ_IDS.hpOn, value: 0, normalized: 0, text: "Off" });
  values.push({ id: EQ_IDS.hpFreq, value: 80, normalized: 0, text: "" });
  return rackSlotDto({ module: `${EQ_MODULE_ID}@1.0.0`, module_id: EQ_MODULE_ID, name: "Parametric EQ", values });
}

afterEach(() => {
  clearMocks();
  resetRackForTest();
});

describe("Add EQ band here (H-42)", () => {
  it("finds the EQ slot", () => {
    const gain = rackSlotDto({ module_id: "org.powervoice.gain" });
    expect(eqSlotIndex(rackStateDto([gain, eqSlot()]))).toBe(1);
    expect(eqSlotIndex(rackStateDto([gain]))).toBeNull();
  });

  it("puts a hum notch on the free band nearest its frequency", () => {
    const plan = planEqAction(eqSlot(), { kind: "notch", freqHz: 50, gainDb: -20, q: 20 })!;
    expect(plan.band).toBe(1);
    expect(plan.sets).toEqual([
      { id: 31, value: 50 },
      { id: 33, value: 20 },
      { id: 32, value: -20 },
      { id: 30, value: 1 },
    ]);
    // Band 1 already in use → the next nearest free one (500 Hz, band 2).
    expect(planEqAction(eqSlot([-4, 0, 0, 0, 0]), { kind: "notch", freqHz: 50, gainDb: -20, q: 20 })!.band).toBe(2);
    // A 3.5 kHz cut lands on the 3 kHz band.
    expect(planEqAction(eqSlot(), { kind: "cut", freqHz: 3500, gainDb: -3, q: 2 })!.band).toBe(4);
  });

  it("turns a rumble finding into the HP band", () => {
    expect(planEqAction(eqSlot([1, 1, 1, 1, 1]), { kind: "high_pass", freqHz: 80, gainDb: 0, q: 0.7 })).toEqual({
      band: "hp",
      sets: [
        { id: 11, value: 80 },
        { id: 10, value: 1 },
      ],
    });
  });

  it("reports when every peak band is taken", () => {
    expect(planEqAction(eqSlot([1, -2, 3, -1, 2]), { kind: "cut", freqHz: 300, gainDb: -3, q: 1.4 })).toBeNull();
  });

  it("issues rack_add then param_set_plain through the rack commands", async () => {
    const calls: Array<{ cmd: string; args: Record<string, unknown> }> = [];
    let state: RackStateDto = rackStateDto([]);
    mockIPC((cmd, args) => {
      calls.push({ cmd, args: (args ?? {}) as Record<string, unknown> });
      if (cmd === "rack_add") {
        state = rackStateDto([eqSlot()]);
        return state;
      }
      if (cmd === "param_set_plain") {
        return state;
      }
      return null;
    });
    const result = await applyEqAction({ kind: "notch", freqHz: 60, gainDb: -20, q: 20 });
    expect(result).toEqual({ outcome: "applied", band: 1 });
    expect(calls.map((c) => c.cmd)).toEqual(["rack_add", "param_set_plain", "param_set_plain", "param_set_plain", "param_set_plain"]);
    expect(calls[0]!.args).toEqual({ moduleId: EQ_MODULE_ID, index: 0 });
    expect(calls.slice(1).map((c) => [c.args.slot, c.args.id, c.args.value])).toEqual([
      [0, 31, 60],
      [0, 33, 20],
      [0, 32, -20],
      [0, 30, 1],
    ]);
  });

  it("uses the existing EQ and sends nothing when no band is free", async () => {
    const calls: string[] = [];
    const full = rackStateDto([eqSlot([1, 1, 1, 1, 1])]);
    mockIPC((cmd) => {
      calls.push(cmd);
      return cmd === "rack_get" ? full : null;
    });
    const result = await applyEqAction({ kind: "cut", freqHz: 300, gainDb: -3, q: 1.4 }, full);
    expect(result).toEqual({ outcome: "no_free_band" });
    expect(calls).toEqual([]);
  });
});

describe("previewEqOverrides (H-101, the Explain modal's dashed EQ-suggestion overlay)", () => {
  it("maps a single boost/cut/notch suggestion onto peak band 1", () => {
    expect(previewEqOverrides([{ kind: "cut", freqHz: 300, gainDb: -3, q: 1.4 }])).toEqual([
      { id: 31, value: 300 },
      { id: 33, value: 1.4 },
      { id: 32, value: -3 },
      { id: 30, value: 1 },
    ]);
  });

  it("maps a high_pass suggestion onto the HP band, independent of peak-band order", () => {
    expect(
      previewEqOverrides([
        { kind: "high_pass", freqHz: 80, gainDb: 0, q: 0.7 },
        { kind: "boost", freqHz: 3000, gainDb: 2, q: 1 },
      ]),
    ).toEqual([
      { id: 11, value: 80 },
      { id: 10, value: 1 },
      { id: 31, value: 3000 },
      { id: 33, value: 1 },
      { id: 32, value: 2 },
      { id: 30, value: 1 },
    ]);
  });

  it("takes peak bands in order for successive suggestions, never reusing one", () => {
    const overrides = previewEqOverrides([
      { kind: "cut", freqHz: 300, gainDb: -3, q: 1.4 },
      { kind: "boost", freqHz: 3000, gainDb: 2, q: 1 },
    ]);
    const bandOf = (id: number): number => EQ_IDS.peaks.findIndex((ids) => ids.freq === id);
    const freqSets = overrides.filter((s) => EQ_IDS.peaks.some((ids) => ids.freq === s.id));
    expect(freqSets.map((s) => bandOf(s.id))).toEqual([0, 1]);
  });

  it("drops suggestions beyond the fifth peak band rather than overwriting one", () => {
    const many = Array.from({ length: 7 }, (_, i) => ({
      kind: "boost" as const,
      freqHz: 300 * (i + 1),
      gainDb: 1,
      q: 1,
    }));
    const overrides = previewEqOverrides(many);
    // 5 peak bands × 4 params each = 20 sets; nothing for suggestions 6 and 7.
    expect(overrides.length).toBe(20);
  });

  it("is empty for no suggestions", () => {
    expect(previewEqOverrides([])).toEqual([]);
  });
});

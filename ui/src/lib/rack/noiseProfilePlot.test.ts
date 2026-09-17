import { describe, expect, it } from "vitest";
import { paramInfoDto, rackSlotDto } from "../test/fixtures";
import {
  NR_GRAPH_DEFAULT_MAX_DB,
  NR_GRAPH_DEFAULT_MIN_DB,
  liveBandFreqsHz,
  noiseProfileFreqRange,
  paramValueByKey,
  profileDbRange,
  reducedToLevelsDb,
} from "./noiseProfilePlot";

/** H-85 (SPEC-014 §2.8 item 2, §4.10): the profile graph's pure geometry/arithmetic, kept
 * separate from the canvas so it's testable under jsdom. */

describe("noiseProfileFreqRange (SPEC-014 §2.8: 20 Hz to min(Nyquist, 24 kHz))", () => {
  it("caps at 24 kHz for a 48 kHz rack", () => {
    expect(noiseProfileFreqRange(48_000)).toEqual([20, 24_000]);
  });

  it("follows the Nyquist rate below 48 kHz", () => {
    expect(noiseProfileFreqRange(32_000)).toEqual([20, 16_000]);
  });

  it("falls back to the full cap for an unknown rate", () => {
    expect(noiseProfileFreqRange(0)).toEqual([20, 24_000]);
  });
});

describe("profileDbRange (SPEC-014 §2.8: −120…0 dBFS default, auto-fit ±10 dB)", () => {
  it("keeps the default range with no levels", () => {
    expect(profileDbRange([])).toEqual([NR_GRAPH_DEFAULT_MIN_DB, NR_GRAPH_DEFAULT_MAX_DB]);
  });

  it("narrows to the print's range plus a 10 dB margin", () => {
    expect(profileDbRange([-80, -60, -70])).toEqual([-90, -50]);
  });

  it("never widens past the default range", () => {
    expect(profileDbRange([-200, 20])).toEqual([NR_GRAPH_DEFAULT_MIN_DB, NR_GRAPH_DEFAULT_MAX_DB]);
  });

  it("ignores empty-band sentinels (−150 dB) so a quiet top octave doesn't collapse the fit", () => {
    expect(profileDbRange([-80, -60, -150, -150])).toEqual([-90, -50]);
  });

  it("ignores non-finite levels", () => {
    expect(profileDbRange([-80, Number.NaN, -Infinity])).toEqual([-90, -70]);
  });
});

describe("reducedToLevelsDb (SPEC-014 §2.8: print − reduction_db × amount_pct/100)", () => {
  it("subtracts the full reduction at 100% amount", () => {
    expect(reducedToLevelsDb([-40, -60], 12, 100)).toEqual([-52, -72]);
  });

  it("scales by the amount percentage", () => {
    expect(reducedToLevelsDb([-40], 12, 50)).toEqual([-46]);
  });

  it("is a no-op at 0% amount", () => {
    expect(reducedToLevelsDb([-40, -60], 12, 0)).toEqual([-40, -60]);
  });
});

describe("paramValueByKey", () => {
  it("reads the slot's current value for the keyed parameter", () => {
    const params = [
      paramInfoDto({ id: 0, key: "reduction_db", default: 12 }),
      paramInfoDto({ id: 1, key: "amount_pct", default: 100 }),
    ];
    const slot = rackSlotDto({
      params,
      values: [
        { id: 0, value: 18, normalized: 0.5, text: "18.0 dB" },
        { id: 1, value: 40, normalized: 0.4, text: "40%" },
      ],
    });
    expect(paramValueByKey(slot, "reduction_db")).toBe(18);
    expect(paramValueByKey(slot, "amount_pct")).toBe(40);
  });

  it("falls back to the schema default when no value is mirrored yet", () => {
    const params = [paramInfoDto({ id: 0, key: "reduction_db", default: 12 })];
    const slot = rackSlotDto({ params, values: [] });
    expect(paramValueByKey(slot, "reduction_db")).toBe(12);
  });

  it("falls back to the given default for an unknown key", () => {
    const slot = rackSlotDto({ params: [], values: [] });
    expect(paramValueByKey(slot, "reduction_db", -1)).toBe(-1);
  });
});

describe("liveBandFreqsHz (H-87: the live analyzer frame's 1/24-octave band centres)", () => {
  it("builds the band ladder from f0 and bands-per-octave", () => {
    const freqs = liveBandFreqsHz(4, 20, 24);
    expect(freqs[0]).toBeCloseTo(20, 6);
    expect(freqs[1]).toBeCloseTo(20 * 2 ** (1 / 24), 6);
    expect(freqs[2]).toBeCloseTo(20 * 2 ** (2 / 24), 6);
    expect(freqs[3]).toBeCloseTo(20 * 2 ** (3 / 24), 6);
  });

  it("returns an empty array for zero bands", () => {
    expect(liveBandFreqsHz(0, 20, 24)).toEqual([]);
  });
});

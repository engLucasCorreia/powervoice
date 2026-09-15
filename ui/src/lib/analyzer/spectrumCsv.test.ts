import { describe, expect, it } from "vitest";
import { CSV_HEADER, spectrumCsv } from "./spectrumCsv";

describe("spectrum CSV export (H-42)", () => {
  it("writes one machine-readable row per point in range", () => {
    const csv = spectrumCsv({ freqsHz: [0, 2.9296875, 100, 20_000], levelsDb: [-3.14159, -Infinity, -20.5, -90] }, 1, 1000);
    expect(csv).toBe(`${CSV_HEADER}\n2.93,\n100.00,-20.50\n`);
  });

  it("defaults to the whole curve", () => {
    const csv = spectrumCsv({ freqsHz: [10, 20], levelsDb: [-1, -2] });
    expect(csv.trim().split("\n")).toEqual([CSV_HEADER, "10.00,-1.00", "20.00,-2.00"]);
  });
});

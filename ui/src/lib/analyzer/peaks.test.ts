import { describe, expect, it } from "vitest";
import { findPeaks } from "./peaks";

const FS = 48_000;
const FFT = 16_384;
const BIN_HZ = FS / FFT;

/** A linear-bin spectrum: a −90 dB floor plus parabolic (in dB) main lobes, like a windowed
 * sine's, at the given true (off-bin) frequencies and levels. */
function binSpectrum(tones: Array<[number, number]>): { freqsHz: Float64Array; levelsDb: Float32Array } {
  const bins = FFT / 2 + 1;
  const freqsHz = new Float64Array(bins);
  const levelsDb = new Float32Array(bins).fill(-90);
  for (let k = 0; k < bins; k++) {
    freqsHz[k] = k * BIN_HZ;
  }
  for (const [f, level] of tones) {
    const centre = f / BIN_HZ;
    for (let k = Math.floor(centre) - 3; k <= Math.ceil(centre) + 3; k++) {
      const d = k - centre;
      const v = level - 6 * d * d;
      if (k >= 0 && k < bins) {
        levelsDb[k] = Math.max(levelsDb[k] ?? -90, v);
      }
    }
  }
  return { freqsHz, levelsDb };
}

describe("findPeaks (H-42)", () => {
  it("finds synthetic multi-sine peaks at their frequency and level, loudest first", () => {
    const curve = binSpectrum([
      [200.4, -20],
      [1000.7, -30],
      [5012.3, -40],
      [9000, -50],
    ]);
    const peaks = findPeaks(curve);
    expect(peaks.map((p) => Math.round(p.freqHz))).toEqual([200, 1001, 5012, 9000]);
    const expected = [
      [200.4, -20],
      [1000.7, -30],
      [5012.3, -40],
      [9000, -50],
    ];
    peaks.forEach((p, i) => {
      const [f, level] = expected[i]!;
      expect(Math.abs(p.freqHz - f!)).toBeLessThan(0.05);
      expect(Math.abs(p.levelDb - level!)).toBeLessThan(0.01);
      expect(p.prominenceDb).toBeGreaterThan(20);
    });
  });

  it("respects the minimum spacing: a quieter neighbour within 1/6 octave is dropped", () => {
    const curve = binSpectrum([
      [1000, -30],
      [1060, -33], // 0.084 octave away
      [1400, -36], // 0.49 octave away
    ]);
    const peaks = findPeaks(curve);
    expect(peaks.map((p) => Math.round(p.freqHz))).toEqual([1000, 1400]);
    const all = findPeaks(curve, { minSpacingOct: 0.05 });
    expect(all.map((p) => Math.round(p.freqHz))).toEqual([1000, 1060, 1400]);
    for (let i = 0; i < peaks.length; i++) {
      for (let j = i + 1; j < peaks.length; j++) {
        expect(Math.abs(Math.log2(peaks[i]!.freqHz / peaks[j]!.freqHz))).toBeGreaterThanOrEqual(1 / 6);
      }
    }
  });

  it("keeps only the requested count, above the floor, inside the range", () => {
    const curve = binSpectrum([
      [100, -20],
      [300, -25],
      [900, -30],
      [2700, -35],
      [8100, -70],
    ]);
    expect(findPeaks(curve, { count: 2 }).map((p) => Math.round(p.freqHz))).toEqual([100, 300]);
    expect(findPeaks(curve, { floorDb: -60 }).length).toBe(4);
    expect(findPeaks(curve, { fMinHz: 200, fMaxHz: 1000 }).map((p) => Math.round(p.freqHz))).toEqual([
      300, 900,
    ]);
  });

  it("ignores ripple that isn't prominent", () => {
    const bins = 2000;
    const freqsHz = Float64Array.from({ length: bins }, (_, k) => (k + 1) * 10);
    const levelsDb = Float32Array.from({ length: bins }, (_, k) => -60 + 2 * Math.sin(k));
    expect(findPeaks({ freqsHz, levelsDb })).toEqual([]);
    expect(findPeaks({ freqsHz: [1, 2], levelsDb: [0, 0] })).toEqual([]);
  });

  it("works on log-spaced bands too (1/24 octave)", () => {
    const n = 246;
    const freqsHz = Float64Array.from({ length: n }, (_, k) => 20 * 2 ** (k / 24));
    const target = 440;
    const centre = 24 * Math.log2(target / 20);
    const levelsDb = Float32Array.from({ length: n }, (_, k) => Math.max(-80, -20 - 3 * (k - centre) ** 2));
    const [peak] = findPeaks({ freqsHz, levelsDb });
    expect(peak).toBeDefined();
    expect(Math.abs(1200 * Math.log2(peak!.freqHz / target))).toBeLessThan(5);
  });
});

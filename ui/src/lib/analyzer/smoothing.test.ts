import { describe, expect, it } from "vitest";
import { smoothFractionalOctave, smoothingOctaves } from "./smoothing";

const bins = (n: number, binHz: number) => Float64Array.from({ length: n }, (_, k) => k * binHz);

describe("fractional-octave smoothing (H-42)", () => {
  it("maps the preference to a window", () => {
    expect(smoothingOctaves("none")).toBeNull();
    expect(smoothingOctaves("third")).toBeCloseTo(1 / 3);
    expect(smoothingOctaves("sixth")).toBeCloseTo(1 / 6);
    expect(smoothingOctaves("twelfth")).toBeCloseTo(1 / 12);
  });

  it("leaves a flat spectrum flat", () => {
    const f = bins(4097, 5.859375);
    const l = new Float32Array(4097).fill(-40);
    const s = smoothFractionalOctave(f, l, 1 / 3);
    for (let k = 1; k < s.length; k++) {
      expect(s[k]).toBeCloseTo(-40, 4);
    }
    expect(s[0]).toBe(-40);
  });

  it("spreads a single line over its window, conserving power", () => {
    const binHz = 1;
    const f = bins(20_001, binHz);
    const l = new Float32Array(20_001).fill(-Infinity);
    l[10_000] = 0; // one line at 10 kHz, 0 dB
    const s = smoothFractionalOctave(f, l, 1 / 3);
    // At 10 kHz the 1/3-octave window spans 8 909 … 11 225 Hz: 2 317 bins.
    const lo = Math.ceil(10_000 / 2 ** (1 / 6));
    const hi = Math.floor(10_000 * 2 ** (1 / 6));
    expect(s[10_000]).toBeCloseTo(-10 * Math.log10(hi - lo + 1), 1);
    // Far from the line: nothing.
    expect(s[1000]).toBe(-Infinity);
  });

  it("is the power mean, not the dB mean", () => {
    const f = Float64Array.from([100, 101, 102]);
    const l = Float32Array.from([0, -Infinity, -Infinity]);
    const s = smoothFractionalOctave(f, l, 1);
    expect(s[1]).toBeCloseTo(10 * Math.log10(1 / 3), 4);
  });
});

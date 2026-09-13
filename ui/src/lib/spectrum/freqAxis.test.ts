import { describe, expect, it } from "vitest";
import {
  clampFreqRange,
  formatHoverFreqHz,
  formatRulerFreqHz,
  freqForU,
  freqForY,
  frequencyTicks,
  fullFreqRange,
  LOG_MIN_HZ,
  panFreqRange,
  uForFreq,
  yForFreq,
  zoomFreqRange,
} from "./freqAxis";

describe("freqAxis (SPEC-007 §2.4, §4.7, AC-11)", () => {
  it("log mode: 20 Hz maps to the bottom (u=0/y=height) and Nyquist to the top (u=1/y=0)", () => {
    const [fLo, fHi] = fullFreqRange("log", 24_000);
    expect(fLo).toBe(LOG_MIN_HZ);
    expect(uForFreq(fLo, fLo, fHi, "log")).toBeCloseTo(0, 6);
    expect(uForFreq(fHi, fLo, fHi, "log")).toBeCloseTo(1, 6);
    expect(yForFreq(fLo, 300, fLo, fHi, "log")).toBeCloseTo(300, 6);
    expect(yForFreq(fHi, 300, fLo, fHi, "log")).toBeCloseTo(0, 6);
  });

  it("linear mode: 0 Hz maps to the bottom and Nyquist to the top", () => {
    const [fLo, fHi] = fullFreqRange("linear", 24_000);
    expect(fLo).toBe(0);
    expect(uForFreq(0, fLo, fHi, "linear")).toBeCloseTo(0, 6);
    expect(uForFreq(fHi, fLo, fHi, "linear")).toBeCloseTo(1, 6);
  });

  it("log and linear f(u) are monotonic", () => {
    const [fLo, fHi] = [20, 20_000];
    for (const scale of ["log", "linear"] as const) {
      let prev = -Infinity;
      for (let u = 0; u <= 1.0001; u += 0.05) {
        const f = freqForU(Math.min(u, 1), fLo, fHi, scale);
        expect(f).toBeGreaterThanOrEqual(prev);
        prev = f;
      }
    }
  });

  it("y -> f -> y round-trips within 0.01 px (AC-11)", () => {
    const [fLo, fHi] = [20, 24_000];
    const heightPx = 437;
    for (const scale of ["log", "linear"] as const) {
      for (let y = 0; y <= heightPx; y += 7) {
        const f = freqForY(y, heightPx, fLo, fHi, scale);
        const y2 = yForFreq(f, heightPx, fLo, fHi, scale);
        expect(Math.abs(y2 - y)).toBeLessThanOrEqual(0.01);
      }
    }
  });

  it("clamps a requested range to the axis floor, Nyquist, and the minimum span", () => {
    const [lo1, hi1] = clampFreqRange(-100, 5, "log", 24_000); // 1 octave min, floor 20
    expect(lo1).toBeCloseTo(20, 6);
    expect(hi1).toBeCloseTo(40, 6);
    expect(clampFreqRange(0, 100, "linear", 24_000)).toEqual([0, 500]); // 500 Hz min span
    expect(clampFreqRange(10, 100_000, "log", 24_000)).toEqual([20, 24_000]);
  });

  it("zooms around an anchor frequency and clamps to the minimum span", () => {
    const nyq = 24_000;
    const [fLo, fHi] = [20, nyq];
    // Zooming in (factor < 1) around 1000 Hz shrinks the range but keeps 1000 Hz fixed under the
    // same relative position (log-symmetric around the anchor).
    const [lo, hi] = zoomFreqRange(fLo, fHi, "log", 1000, 0.5, nyq);
    expect(lo).toBeGreaterThan(fLo);
    expect(hi).toBeLessThan(fHi);
    expect(lo).toBeLessThan(1000);
    expect(hi).toBeGreaterThan(1000);
    // Zooming in repeatedly never goes below the 1-octave minimum.
    let range: [number, number] = [fLo, fHi];
    for (let i = 0; i < 100; i++) {
      range = zoomFreqRange(range[0], range[1], "log", 1000, 0.5, nyq);
    }
    expect(range[1] / range[0]).toBeGreaterThanOrEqual(2 - 1e-9);
  });

  it("pans a range toward higher frequency without exceeding Nyquist", () => {
    const nyq = 24_000;
    const [lo, hi] = panFreqRange(1000, 2000, "log", 10, nyq); // huge pan clamps at Nyquist
    expect(hi).toBe(nyq);
    expect(lo).toBeLessThan(hi);
  });

  it("formats ruler labels per the 1-2-5 ladder", () => {
    expect(formatRulerFreqHz(20)).toBe("20");
    expect(formatRulerFreqHz(500)).toBe("500");
    expect(formatRulerFreqHz(1000)).toBe("1k");
    expect(formatRulerFreqHz(2000)).toBe("2k");
    expect(formatRulerFreqHz(2500)).toBe("2.5k");
    expect(formatRulerFreqHz(12_000)).toBe("12k");
    expect(formatRulerFreqHz(20_000)).toBe("20k");
  });

  it("formats the hover frequency text (SPEC-007 §2.7)", () => {
    expect(formatHoverFreqHz(1007.8125)).toBe("1 007.8 Hz");
    expect(formatHoverFreqHz(12_350)).toBe("12.35 kHz");
  });

  it("never overlaps ruler labels at pane heights 80-2000 px (AC-11)", () => {
    const [fLo, fHi] = fullFreqRange("log", 48_000);
    for (const heightPx of [80, 150, 300, 600, 1200, 2000]) {
      const ticks = frequencyTicks(fLo, fHi, "log", heightPx, 24);
      for (let i = 1; i < ticks.length; i++) {
        expect(Math.abs(ticks[i]!.y - ticks[i - 1]!.y)).toBeGreaterThanOrEqual(24 - 1e-9);
      }
    }
  });

  it("shows a single hot bin's row exactly via a tick at its frequency (sanity: ticks land on the ladder)", () => {
    const ticks = frequencyTicks(20, 20_000, "log", 300, 20);
    expect(ticks.some((t) => t.freqHz === 20)).toBe(true);
    expect(ticks.some((t) => t.freqHz === 20_000)).toBe(true);
  });
});

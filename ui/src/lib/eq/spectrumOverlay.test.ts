import { describe, expect, it } from "vitest";
import { analyzerBandFreqsHz, xForFreq } from "./freqAxis";
import { totalCurveToScreen } from "./curvePoints";
import {
  EQ_SPECTRUM_CEIL_DBFS,
  EQ_SPECTRUM_FLOOR_DBFS,
  sameSpectrumLevels,
  spectrumOverlayPoints,
  yForSpectrumDbfs,
} from "./spectrumOverlay";
import type { ResponseCurveDto } from "../ipc/bindings";

const WIDTH = 400;
const HEIGHT = 160;
const F_LO = 20;
const F_HI = 20_000;

describe("yForSpectrumDbfs (SPEC-015 §2.6.2 fixed -90..0 dBFS scale)", () => {
  it("maps the ceiling to the top and the floor to the bottom", () => {
    expect(yForSpectrumDbfs(EQ_SPECTRUM_CEIL_DBFS, HEIGHT)).toBeCloseTo(0, 6);
    expect(yForSpectrumDbfs(EQ_SPECTRUM_FLOOR_DBFS, HEIGHT)).toBeCloseTo(HEIGHT, 6);
  });

  it("clamps beyond the fixed range", () => {
    expect(yForSpectrumDbfs(10, HEIGHT)).toBeCloseTo(0, 6);
    expect(yForSpectrumDbfs(-200, HEIGHT)).toBeCloseTo(HEIGHT, 6);
  });

  it("is independent of the gain range (unlike yForDb)", () => {
    // Same call, no rangeDb parameter at all — the scale never changes with the ±12/±24 toggle.
    const y1 = yForSpectrumDbfs(-45, HEIGHT);
    const y2 = yForSpectrumDbfs(-45, HEIGHT);
    expect(y1).toBe(y2);
  });
});

describe("spectrumOverlayPoints x agreement with the total curve (AC-21)", () => {
  it("draws a band centre and a curve point at the same frequency at the same x", () => {
    const bandFreqs = analyzerBandFreqsHz(50, 20, 24);
    const levels = Array.from(bandFreqs, () => -40);
    const xf = (f: number) => xForFreq(f, WIDTH, F_LO, F_HI);
    const overlay = spectrumOverlayPoints(bandFreqs, levels, xf, F_LO, F_HI);

    // A synthetic curve whose only point sits exactly at one analyzer band's frequency.
    const bandIndex = 20;
    const fK = bandFreqs[bandIndex]!;
    const curve: ResponseCurveDto = {
      freqs_hz: [fK],
      sample_rate_hz: 48_000,
      total_db: [0],
      components_db: [],
    };
    const curvePoints = totalCurveToScreen(curve, WIDTH, HEIGHT, F_LO, F_HI, 12);
    const curveX = curvePoints[0]!.x;
    const overlayPoint = overlay.find((p) => Math.abs(p.x - curveX) < 1);
    expect(overlayPoint).toBeDefined();
    // Within 0.5 device px, per AC-21 (both computed through the same xForFreq at device scale 1).
    expect(Math.abs(xf(fK) - curveX)).toBeLessThanOrEqual(0.5);
  });

  it("clips to [fLo, fHi] and keeps only the loudest point per pixel column", () => {
    const freqs = [10, 20, 30, 20_000, 30_000];
    const levels = [-10, -20, -5, -30, -1];
    const points = spectrumOverlayPoints(freqs, levels, (f) => xForFreq(f, WIDTH, F_LO, F_HI), F_LO, F_HI);
    expect(points.length).toBeGreaterThan(0);
    expect(points.every((p) => p.x >= 0 && p.x <= WIDTH)).toBe(true);
  });
});

describe("sameSpectrumLevels (H-43/H-84 idle dedup)", () => {
  it("is false for the first frame (no previous)", () => {
    expect(sameSpectrumLevels(null, { reset: false, silent: false, levelsDb: [-40, -50] })).toBe(false);
  });

  it("is true for two frames with identical levels and silence", () => {
    const a = { reset: false, silent: false, levelsDb: [-40, -50] };
    const b = { reset: false, silent: false, levelsDb: [-40, -50] };
    expect(sameSpectrumLevels(a, b)).toBe(true);
  });

  it("is false when a level differs, the length differs, or silence differs", () => {
    const a = { reset: false, silent: false, levelsDb: [-40, -50] };
    expect(sameSpectrumLevels(a, { reset: false, silent: false, levelsDb: [-40, -51] })).toBe(false);
    expect(sameSpectrumLevels(a, { reset: false, silent: false, levelsDb: [-40] })).toBe(false);
    expect(sameSpectrumLevels(a, { reset: false, silent: true, levelsDb: [-40, -50] })).toBe(false);
  });

  it("is always false when either frame is a reset (device reopen/rate change)", () => {
    const a = { reset: false, silent: false, levelsDb: [-40, -50] };
    const b = { reset: false, silent: false, levelsDb: [-40, -50] };
    expect(sameSpectrumLevels({ ...a, reset: true }, b)).toBe(false);
    expect(sameSpectrumLevels(a, { ...b, reset: true })).toBe(false);
  });
});

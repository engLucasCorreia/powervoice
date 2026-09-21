import { describe, expect, it } from "vitest";
import { rectsOverlap } from "../../ui/axisLabels";
import {
  computeDbRange,
  eqAdviceLevels,
  eqAdviceRequestFreqs,
  EQ_ADVICE_MAX_POINTS,
  f0LabelTopPx,
  markerReservedRects,
} from "./explainGraphMath";

describe("computeDbRange (H-92: never truncate a real peak)", () => {
  it("gives every finite point headroom above and below, on a clean 10 dB grid", () => {
    const raw = Float32Array.from([-90, -40, -95, -Infinity]);
    const smoothed = Float32Array.from([-88, -45, -90]);
    const [floor, ceil] = computeDbRange(raw, smoothed);
    expect(ceil).toBeGreaterThanOrEqual(-40 + 6);
    expect(floor).toBeLessThanOrEqual(-95 - 6);
    expect(Math.abs(ceil % 10)).toBe(0);
    expect(Math.abs(floor % 10)).toBe(0);
  });

  it("never truncates the loudest real point of either curve", () => {
    const raw = Float32Array.from([-12, -60, -80]);
    const smoothed = Float32Array.from([-70, -75]);
    const [, ceil] = computeDbRange(raw, smoothed);
    expect(ceil).toBeGreaterThan(-12);
  });

  it("keeps a minimum span even for a nearly flat curve", () => {
    const raw = Float32Array.from([-40, -40.2, -39.9]);
    const [floor, ceil] = computeDbRange(raw, raw);
    expect(ceil - floor).toBeGreaterThanOrEqual(30);
  });

  it("falls back to the analyzer's usual range when nothing is finite", () => {
    const silence = Float32Array.from([-Infinity, -Infinity]);
    expect(computeDbRange(silence, silence)).toEqual([-120, 0]);
  });
});

const PLOT = { x: 48, y: 10, width: 400, height: 200 };

describe("markerReservedRects (H-102: annotation cards must never sit on a marker or the band-label row)", () => {
  it("is empty when nothing is drawn", () => {
    expect(
      markerReservedRects({ plot: PLOT, f0X: null, harmonics: [], strongestPeak: null, showBandLabels: false }),
    ).toEqual([]);
  });

  it("reserves a strip across the whole plot width for the band-label row", () => {
    const rects = markerReservedRects({ plot: PLOT, f0X: null, harmonics: [], strongestPeak: null, showBandLabels: true });
    expect(rects).toHaveLength(1);
    expect(rects[0]!.x).toBe(PLOT.x);
    expect(rects[0]!.width).toBe(PLOT.width);
    expect(rects[0]!.y).toBe(PLOT.y);
  });

  it("reserves a box around the F0 label, anchored at the given x", () => {
    const rects = markerReservedRects({ plot: PLOT, f0X: 120, harmonics: [], strongestPeak: null, showBandLabels: false });
    expect(rects).toHaveLength(1);
    expect(rects[0]!.x).toBeLessThanOrEqual(120);
    expect(rects[0]!.x + rects[0]!.width).toBeGreaterThanOrEqual(120);
  });

  it("reserves one box per harmonic marker, around its own point", () => {
    const rects = markerReservedRects({
      plot: PLOT,
      f0X: null,
      harmonics: [{ x: 120, y: 60 }, { x: 240, y: 90 }],
      strongestPeak: null,
      showBandLabels: false,
    });
    expect(rects).toHaveLength(2);
    for (const [i, h] of [{ x: 120, y: 60 }, { x: 240, y: 90 }].entries()) {
      expect(rects[i]!.x).toBeLessThanOrEqual(h.x);
      expect(rects[i]!.x + rects[i]!.width).toBeGreaterThanOrEqual(h.x);
    }
  });

  it("reserves a box around the strongest-peak marker", () => {
    const rects = markerReservedRects({ plot: PLOT, f0X: null, harmonics: [], strongestPeak: { x: 300, y: 50 }, showBandLabels: false });
    expect(rects).toHaveLength(1);
    expect(rects[0]!.x).toBeLessThanOrEqual(300);
    expect(rects[0]!.x + rects[0]!.width).toBeGreaterThanOrEqual(300);
  });

  it("the F0 reservation and a same-frequency H1 reservation overlap — exactly the collision H-102 found, which the caller must offset before calling this", () => {
    const rects = markerReservedRects({
      plot: PLOT,
      f0X: 120,
      harmonics: [{ x: 120, y: PLOT.y + 8 }],
      strongestPeak: null,
      showBandLabels: false,
    });
    expect(rects).toHaveLength(2);
    expect(rectsOverlap(rects[0]!, rects[1]!)).toBe(true);
  });
});

describe("f0LabelTopPx (H-102: the F0 label must never sit on the band-name row)", () => {
  it("sits at the plot's own top when no band-name row is drawn", () => {
    expect(f0LabelTopPx(PLOT, false)).toBe(PLOT.y);
  });

  it("drops below the band-name row when it is drawn — a voice's F0 sits inside 'Fundamental'", () => {
    const top = f0LabelTopPx(PLOT, true);
    expect(top).toBeGreaterThan(PLOT.y);
  });

  it("the F0 reservation no longer overlaps the band-label row reservation once both are drawn", () => {
    const rects = markerReservedRects({ plot: PLOT, f0X: 120, harmonics: [], strongestPeak: null, showBandLabels: true });
    expect(rects).toHaveLength(2); // band row + F0
    expect(rectsOverlap(rects[0]!, rects[1]!)).toBe(false);
  });
});

describe("eqAdviceRequestFreqs (H-101: the EQ-suggestion preview request)", () => {
  it("returns `columns` ascending points strictly inside [fLo, fHi]", () => {
    const freqs = eqAdviceRequestFreqs(20, 20_000, 16);
    expect(freqs).toHaveLength(16);
    for (const f of freqs) {
      expect(f).toBeGreaterThan(20);
      expect(f).toBeLessThan(20_000);
    }
    expect([...freqs].sort((a, b) => a - b)).toEqual(freqs);
  });

  it("is capped at EQ_ADVICE_MAX_POINTS even for a huge column count", () => {
    expect(eqAdviceRequestFreqs(20, 20_000, 10_000)).toHaveLength(EQ_ADVICE_MAX_POINTS);
  });

  it("is empty for zero columns or a degenerate range", () => {
    expect(eqAdviceRequestFreqs(20, 20_000, 0)).toEqual([]);
    expect(eqAdviceRequestFreqs(20_000, 20, 8)).toEqual([]);
    expect(eqAdviceRequestFreqs(100, 100, 8)).toEqual([]);
  });
});

describe("eqAdviceLevels (H-101: measured spectrum + the previewed filter, never modifying either)", () => {
  it("adds the filter's own response to the measured level at each frequency", () => {
    const freqs = [100, 1_000, 10_000];
    const totalDb = [0, 6, -3];
    const levels = eqAdviceLevels(freqs, totalDb, (f) => (f === 1_000 ? -20 : -30));
    expect(levels).toEqual([-30, -14, -33]);
  });

  it("never mutates its inputs — only ever combines them into a new array", () => {
    const freqs = [1_000];
    const totalDb = [3];
    eqAdviceLevels(freqs, totalDb, () => -20);
    expect(freqs).toEqual([1_000]);
    expect(totalDb).toEqual([3]);
  });

  it("is NaN where the measured level is non-finite, so the caller's line breaks instead of drawing a wrong point", () => {
    const levels = eqAdviceLevels([1_000], [3], () => -Infinity);
    expect(levels[0]).toBeNaN();
  });

  it("is NaN where the previewed response is missing (a shorter totalDb than freqsHz)", () => {
    const levels = eqAdviceLevels([1_000, 2_000], [3], () => -20);
    expect(levels[0]).toBe(-17);
    expect(levels[1]).toBeNaN();
  });
});

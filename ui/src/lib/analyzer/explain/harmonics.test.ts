/// <reference types="node" />
import { describe, expect, it } from "vitest";
import { readFileSync } from "fs";
import { dirname, join } from "path";
import { fileURLToPath } from "url";

import { findPeaks } from "../peaks";
import {
  cents,
  chooseFundamental,
  harmonicBand,
  harmonicNumberOf,
  highestSeparableHarmonic,
  lastResolvableHarmonic,
  measureHarmonics,
  relateStrongestPeak,
  usableRange,
  type PitchRange,
} from "./harmonics";
import { HARMONIC_MIN_PROMINENCE_DB } from "./thresholds";
import { combCurve, type SyntheticCurve } from "./voiceFixtures";

const repoRoot = join(dirname(fileURLToPath(import.meta.url)), "../../../../..");

/** A steady voice: the tracker's percentiles sit a few cents either side of the median. */
function steady(f0Hz: number, spreadCents = 60): PitchRange {
  return {
    medianHz: f0Hz,
    lowHz: f0Hz * 2 ** (-spreadCents / 1200),
    highHz: f0Hz * 2 ** (spreadCents / 1200),
  };
}

function strongestOf(curve: SyntheticCurve) {
  return findPeaks(curve, { count: 8, fMinHz: 20 })[0];
}

function relate(curve: SyntheticCurve, range: PitchRange) {
  const harmonics = measureHarmonics(curve, range);
  return { harmonics, peak: relateStrongestPeak(curve, range, strongestOf(curve), harmonics) };
}

describe("harmonic measurement", () => {
  it("supports the harmonics that are there and not the ones that are not", () => {
    // H1…H4 present, H5 missing, H6 present.
    const f0 = 137;
    const curve = combCurve({
      f0Hz: f0,
      harmonicsDb: [-38, -30, -36, -44, -Infinity, -52],
      floorDb: -95,
    });
    const harmonics = measureHarmonics(curve, steady(f0));
    const supported = harmonics.filter((h) => h.status === "supported").map((h) => h.n);
    expect(supported).toEqual([1, 2, 3, 4, 6]);
    expect(harmonics.find((h) => h.n === 5)?.status).toBe("weak");
    for (const h of harmonics) {
      if (h.status !== "supported") {
        continue;
      }
      // Every reported line is inside the band it was looked for in, and close to where the
      // comb actually put it.
      expect(h.peakHz).not.toBeNull();
      expect(h.peakHz!).toBeGreaterThanOrEqual(h.bandHz[0]);
      expect(h.peakHz!).toBeLessThanOrEqual(h.bandHz[1]);
      expect(Math.abs(cents(h.peakHz!, h.n * f0))).toBeLessThan(30);
      expect(h.prominenceDb!).toBeGreaterThanOrEqual(HARMONIC_MIN_PROMINENCE_DB);
    }
  });

  it("refuses to measure harmonics the speaker's pitch range has smeared together", () => {
    // A range this wide makes neighbouring harmonics overlap from H3 up.
    const wide: PitchRange = { medianHz: 104, lowHz: 87, highHz: 129 };
    const separable = highestSeparableHarmonic(wide, 6);
    expect(separable).toBe(2);
    const curve = combCurve({ f0Hz: 104, harmonicsDb: [-36, -30, -34, -40, -46, -52] });
    const harmonics = measureHarmonics(curve, wide);
    expect(harmonics.filter((h) => h.status === "unresolved").map((h) => h.n)).toEqual([3, 4, 5, 6]);
    expect(lastResolvableHarmonic(wide)).toBe(2);
    // A steady voice resolves every harmonic asked for.
    expect(lastResolvableHarmonic(steady(104))).toBe(6);
  });

  it("widens a degenerate pitch range instead of dividing by zero", () => {
    const flat: PitchRange = { medianHz: 200, lowHz: 200, highHz: 200 };
    const usable = usableRange(flat);
    expect(usable.lowHz).toBeLessThan(200);
    expect(usable.highHz).toBeGreaterThan(200);
    const [low, high] = harmonicBand(3, flat);
    expect(low).toBeLessThan(600);
    expect(high).toBeGreaterThan(600);
  });
});

describe("the strongest peak versus the fundamental", () => {
  it("names the harmonic when the loudest partial is not the fundamental", () => {
    const f0 = 98;
    const curve = combCurve({ f0Hz: f0, harmonicsDb: [-41, -29, -38, -46, -54, -60] });
    const { peak } = relate(curve, steady(f0));
    expect(peak).not.toBeNull();
    expect(peak!.harmonicNumber).toBe(2);
    expect(peak!.isFundamental).toBe(false);
    expect(Math.abs(cents(peak!.impliedF0Hz!, f0))).toBeLessThan(30);
    // The headline number: how much louder the loudest partial is than the fundamental.
    expect(peak!.aboveFundamentalDb!).toBeGreaterThan(8);
    expect(peak!.aboveFundamentalDb!).toBeLessThan(16);
  });

  it("says so when the fundamental really is the strongest", () => {
    const f0 = 210;
    const curve = combCurve({ f0Hz: f0, harmonicsDb: [-28, -37, -44, -50, -56, -62] });
    const { peak } = relate(curve, steady(f0));
    expect(peak!.harmonicNumber).toBe(1);
    expect(peak!.isFundamental).toBe(true);
    expect(Math.abs(peak!.aboveFundamentalDb!)).toBeLessThan(0.5);
  });

  it("does not call a resonance off the comb a harmonic", () => {
    const f0 = 100;
    // A loud resonance at 4.55 × F0 — no integer harmonic can explain it.
    const curve = combCurve({
      f0Hz: f0,
      harmonicsDb: [-40, -38, -42, -48, -54, -60],
      resonances: [[455, -26, 12]],
    });
    const { peak } = relate(curve, steady(f0));
    expect(peak!.freqHz).toBeGreaterThan(440);
    expect(peak!.freqHz).toBeLessThan(470);
    expect(peak!.harmonicNumber).toBeNull();
    expect(peak!.impliedF0Hz).toBeNull();
  });

  it("refuses a harmonic number the pitch range cannot support", () => {
    // With a ±3-semitone range only H1 and H2 are separable, so a loud peak up at H5 is
    // reported as a region, not as "the fifth harmonic".
    const wide: PitchRange = { medianHz: 104, lowHz: 87, highHz: 129 };
    expect(harmonicNumberOf(5 * 104, wide)).toBeNull();
    expect(harmonicNumberOf(2 * 104, wide)).toBe(2);
    expect(harmonicNumberOf(5 * 104, steady(104))).toBe(5);
  });

  it("has no opinion when there are no peaks at all", () => {
    const flat = combCurve({ f0Hz: 100, harmonicsDb: [], floorDb: -90 });
    expect(relateStrongestPeak(flat, steady(100), strongestOf(flat), [])).toBeNull();
  });
});

describe("the octave the fundamental is in", () => {
  it("leaves the tracker's octave alone when the spectrum agrees", () => {
    const f0 = 115;
    const curve = combCurve({ f0Hz: f0, harmonicsDb: [-38, -30, -36, -42, -48, -54] });
    const choice = chooseFundamental(curve, steady(f0));
    expect(choice.checked).toBe(true);
    expect(choice.ratio).toBe(1);
    expect(choice.fundamentalHz).toBe(f0);
    expect(choice.subHarmonics.filter((h) => h.status === "supported")).toHaveLength(0);
  });

  it("moves the fundamental down when the tracker locked onto the second harmonic", () => {
    // The real voice is at 95 Hz; the tracker reports 190. The lines at 1.5 × 190, 2.5 × 190 and
    // 3.5 × 190 (H3, H5, H7 of the real 95 Hz) are the evidence.
    const real = 95;
    const curve = combCurve({
      f0Hz: real,
      harmonicsDb: [-46, -30, -36, -34, -40, -38, -44, -48],
      maxHz: 1200,
    });
    const choice = chooseFundamental(curve, steady(2 * real));
    expect(choice.checked).toBe(true);
    expect(choice.ratio).toBe(0.5);
    expect(Math.abs(cents(choice.fundamentalHz, real))).toBeLessThan(1);
    expect(choice.subHarmonics.filter((h) => h.status === "supported").length).toBeGreaterThanOrEqual(2);
  });

  it("never moves the fundamental up, however weak it is", () => {
    // A missing fundamental: nothing at F0 at all, H2 loudest. The tracker is right (that is
    // the period of the waveform) and the report must not "fix" it to H2.
    const f0 = 92;
    const curve = combCurve({
      f0Hz: f0,
      harmonicsDb: [-Infinity, -30, -35, -40, -46, -52],
    });
    const choice = chooseFundamental(curve, steady(f0));
    expect(choice.ratio).toBe(1);
    expect(choice.fundamentalHz).toBe(f0);
  });

  it("declines to judge when the pitch range is too wide for the sub-harmonics to separate", () => {
    const wide: PitchRange = { medianHz: 104, lowHz: 87, highHz: 129 };
    const curve = combCurve({ f0Hz: 104, harmonicsDb: [-38, -30, -36, -42, -48, -54] });
    const choice = chooseFundamental(curve, wide);
    expect(choice.checked).toBe(false);
    expect(choice.ratio).toBe(1);
    expect(choice.fundamentalHz).toBe(104);
  });
});

describe("the owner's own recording (spectrum.csv)", () => {
  /**
   * `spectrum.csv` in the repo root is the owner's exported spectrum of `ExampleRecording.wav`
   * (31.5 s, 48 kHz mono), and the numbers below are what `vox_dsp`'s offline analysis measures
   * from that same wav — they are measurements, not choices, and the assertions are about the
   * *relationships* between them and the spectrum, so they keep their meaning if the tracker is
   * ever retuned.
   */
  const OWNER_RANGE: PitchRange = { medianHz: 103.55, lowHz: 87.35, highHz: 128.76 };

  function ownerCurve(): SyntheticCurve {
    const text = readFileSync(join(repoRoot, "spectrum.csv"), "utf-8");
    const rows = text.trim().split("\n").slice(1);
    const freqsHz = new Float64Array(rows.length);
    const levelsDb = new Float32Array(rows.length);
    rows.forEach((row, i) => {
      const [f = "0", db = ""] = row.split(",");
      freqsHz[i] = Number(f);
      levelsDb[i] = db === "" ? -Infinity : Number(db);
    });
    return { freqsHz, levelsDb };
  }

  it("is the curve the fixture claims: ascending bins, no NaN", () => {
    const curve = ownerCurve();
    expect(curve.freqsHz.length).toBeGreaterThan(8000);
    for (let i = 1; i < curve.freqsHz.length; i++) {
      expect(curve.freqsHz[i]!).toBeGreaterThan(curve.freqsHz[i - 1]!);
      expect(Number.isNaN(curve.levelsDb[i])).toBe(false);
    }
  });

  it("finds the same strongest peak an independent scan of the file finds", () => {
    const curve = ownerCurve();
    let argmax = 0;
    for (let i = 0; i < curve.levelsDb.length; i++) {
      if (curve.levelsDb[i]! > curve.levelsDb[argmax]!) {
        argmax = i;
      }
    }
    const peak = strongestOf(curve);
    expect(peak).toBeDefined();
    // Within one bin of the raw maximum (the peak is parabolically refined).
    expect(Math.abs(peak!.freqHz - curve.freqsHz[argmax]!)).toBeLessThan(
      curve.freqsHz[1]! - curve.freqsHz[0]!,
    );
  });

  it("reads the loudest partial as the second harmonic, not as the fundamental", () => {
    const curve = ownerCurve();
    const { harmonics, peak } = relate(curve, OWNER_RANGE);
    expect(peak!.harmonicNumber).toBe(2);
    expect(peak!.isFundamental).toBe(false);
    // The implied fundamental is a pitch this speaker actually used.
    expect(peak!.impliedF0Hz!).toBeGreaterThan(OWNER_RANGE.lowHz);
    expect(peak!.impliedF0Hz!).toBeLessThan(OWNER_RANGE.highHz);
    // H2 stands clear of the valleys beside it; H1 does not stand out at all…
    expect(harmonics.find((h) => h.n === 2)?.status).toBe("supported");
    expect(harmonics.find((h) => h.n === 1)?.status).toBe("weak");
    // …and it is the louder of the two by a wide margin.
    expect(peak!.aboveFundamentalDb!).toBeGreaterThan(6);
    // Above H2 this speaker's pitch range has smeared the comb: say so rather than measure it.
    expect(harmonics.filter((h) => h.status === "unresolved").map((h) => h.n)).toEqual([3, 4, 5, 6]);
  });

  it("does not move the octave of a voice the tracker got right", () => {
    const choice = chooseFundamental(ownerCurve(), OWNER_RANGE);
    expect(choice.ratio).toBe(1);
    expect(choice.fundamentalHz).toBe(OWNER_RANGE.medianHz);
  });
});

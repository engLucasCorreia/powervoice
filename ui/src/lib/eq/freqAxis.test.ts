import { describe, expect, it } from "vitest";
import {
  EQ_MAX_HZ,
  EQ_MIN_HZ,
  curveRequestFreqs,
  freqForX,
  graphMaxHz,
  inRange,
  logSpacedFreqs,
  xForFreq,
} from "./freqAxis";

/**
 * Log-frequency axis mapping tests (S3-07, SPEC-015 §2.6.2/§2.6.4): pixel ↔ Hz round-trips
 * exactly (within floating-point tolerance) on the log axis, clamping at the edges, and the
 * curve request's frequency list matches §4.10 (log-spaced columns plus every band's exact
 * frequency, ascending, deduplicated, capped).
 */

describe("graphMaxHz", () => {
  it("caps at 20 kHz for rates at or above 40 kHz", () => {
    expect(graphMaxHz(48_000)).toBe(EQ_MAX_HZ);
    expect(graphMaxHz(44_100)).toBe(EQ_MAX_HZ);
  });

  it("is Nyquist for slower rates (SPEC-015 §2.4)", () => {
    expect(graphMaxHz(8_000)).toBe(4_000);
  });

  it("falls back to 20 kHz for an unknown (<=0) rate", () => {
    expect(graphMaxHz(0)).toBe(EQ_MAX_HZ);
    expect(graphMaxHz(-1)).toBe(EQ_MAX_HZ);
  });
});

describe("xForFreq / freqForX round trip", () => {
  const width = 400;

  it("maps the edges to the pixel edges", () => {
    expect(xForFreq(EQ_MIN_HZ, width, EQ_MIN_HZ, EQ_MAX_HZ)).toBeCloseTo(0, 6);
    expect(xForFreq(EQ_MAX_HZ, width, EQ_MIN_HZ, EQ_MAX_HZ)).toBeCloseTo(width, 6);
  });

  it("round-trips exactly on the log axis", () => {
    for (const f of [20, 50, 100, 440, 1_000, 4_000, 10_000, 19_999]) {
      const x = xForFreq(f, width, EQ_MIN_HZ, EQ_MAX_HZ);
      expect(freqForX(x, width, EQ_MIN_HZ, EQ_MAX_HZ)).toBeCloseTo(f, 3);
    }
  });

  it("1 kHz sits at the geometric-mean pixel (log axis, not linear)", () => {
    // log2(1000/20) / log2(20000/20) = log(50)/log(1000) ≈ 0.567
    const x = xForFreq(1_000, width, EQ_MIN_HZ, EQ_MAX_HZ);
    expect(x / width).toBeCloseTo(Math.log(50) / Math.log(1_000), 4);
  });

  it("clamps out-of-range frequencies to the edges", () => {
    expect(xForFreq(1, width, EQ_MIN_HZ, EQ_MAX_HZ)).toBe(0);
    expect(xForFreq(100_000, width, EQ_MIN_HZ, EQ_MAX_HZ)).toBe(width);
  });

  it("clamps out-of-range pixels to the frequency edges", () => {
    expect(freqForX(-50, width, EQ_MIN_HZ, EQ_MAX_HZ)).toBe(EQ_MIN_HZ);
    expect(freqForX(width + 50, width, EQ_MIN_HZ, EQ_MAX_HZ)).toBe(EQ_MAX_HZ);
  });
});

describe("inRange", () => {
  it("is false exactly at and outside the edges (SPEC-015 §2.6.3 caret)", () => {
    expect(inRange(20, 20, 20_000)).toBe(false);
    expect(inRange(20_000, 20, 20_000)).toBe(false);
    expect(inRange(19, 20, 20_000)).toBe(false);
    expect(inRange(1_000, 20, 20_000)).toBe(true);
  });
});

describe("logSpacedFreqs", () => {
  it("returns `count` ascending points strictly inside the range", () => {
    const pts = logSpacedFreqs(20, 20_000, 8);
    expect(pts).toHaveLength(8);
    for (let i = 1; i < pts.length; i++) {
      expect(pts[i]!).toBeGreaterThan(pts[i - 1]!);
    }
    expect(pts.every((f) => f > 20 && f < 20_000)).toBe(true);
  });

  it("is empty for a non-positive count", () => {
    expect(logSpacedFreqs(20, 20_000, 0)).toEqual([]);
  });
});

describe("curveRequestFreqs (SPEC-015 §4.10 lean form)", () => {
  it("includes the log-spaced columns plus every band frequency, ascending and deduplicated", () => {
    const freqs = curveRequestFreqs(20, 20_000, 4, [1_000, 1_000, 50], 512);
    expect(freqs).toContain(1_000);
    expect(freqs).toContain(50);
    for (let i = 1; i < freqs.length; i++) {
      expect(freqs[i]!).toBeGreaterThan(freqs[i - 1]!); // ascending, deduplicated
    }
  });

  it("drops a band frequency outside [fLo, fHi]", () => {
    const freqs = curveRequestFreqs(20, 20_000, 0, [50_000], 512);
    expect(freqs).not.toContain(50_000);
  });

  it("caps the total at maxPoints", () => {
    const many = Array.from({ length: 600 }, (_, i) => 20 + i);
    const freqs = curveRequestFreqs(20, 20_000, 0, many, 512);
    expect(freqs.length).toBeLessThanOrEqual(512);
  });
});

import { describe, expect, it } from "vitest";
import { autoFftSize, frameColumnBounds, hopForZoom } from "./geometry";
import { sampleGridSpan } from "./sampler";
import { MAX_SHADER_TAPS, pixelDbBounded, sampleGridSpanBounded } from "./shaderSampler";

/**
 * Parity tests (H-13 ticket: "shader math mirrored in TS vs sampler.ts/colormap.ts") for the
 * bounded max-or-interpolate rule the WebGL2 fragment shader implements
 * (`webglRenderer.ts`'s `TILE_FS`). `sampleGridSpanBounded` must agree with the canonical,
 * unbounded `sampleGridSpan` (`sampler.ts`, used by the Canvas2D fallback) whenever the queried
 * span is within the shader's {@link MAX_SHADER_TAPS} cap — which SPEC-007 §4.3's own hop rule
 * guarantees for the time axis in realistic viewports (checked below using the real geometry
 * helpers, not hand-picked numbers).
 */

function makeGrid(count: number, value: (i: number) => number | null): (i: number) => number | null {
  return (i) => (i < 0 || i >= count ? null : value(i));
}

describe("sampleGridSpanBounded parity with sampler.ts's sampleGridSpan", () => {
  const grid = makeGrid(64, (i) => Math.sin(i * 0.3) * 10 - 5);

  it("agrees exactly on the interpolate branch (span < 1 grid step)", () => {
    for (let lo = 0; lo < 60; lo += 0.37) {
      const hi = lo + 0.6;
      expect(sampleGridSpanBounded(64, lo, hi, grid)).toBe(sampleGridSpan(64, lo, hi, grid));
    }
  });

  it("agrees exactly on the max branch when the span is within the tap cap", () => {
    for (let lo = 0; lo < 50; lo += 1) {
      const hi = lo + 5; // well under MAX_SHADER_TAPS
      expect(sampleGridSpanBounded(64, lo, hi, grid)).toBe(sampleGridSpan(64, lo, hi, grid));
    }
  });

  it("agrees at exactly the tap cap boundary", () => {
    const hi = 0 + MAX_SHADER_TAPS; // exactly 16 grid points covered: [0, 16)
    expect(sampleGridSpanBounded(64, 0, hi, grid)).toBe(sampleGridSpan(64, 0, hi, grid));
  });

  it("documents the accepted approximation past the tap cap: the bounded max only scans the first MAX_SHADER_TAPS points", () => {
    // A spike far into the tail of a wide span: the unbounded max finds it, the bounded one (which
    // only scans the first 16 points from the span's low edge) misses it.
    const spikyGrid = makeGrid(64, (i) => (i === 40 ? 100 : 0));
    const lo = 0;
    const hi = 50; // 50 grid points — far past the 16-tap cap.
    const full = sampleGridSpan(64, lo, hi, spikyGrid);
    const bounded = sampleGridSpanBounded(64, lo, hi, spikyGrid);
    expect(full).toBe(100);
    expect(bounded).not.toBe(100);
    expect(bounded).toBe(0); // taps 0..15 are all 0 in this fixture.
    // The bounded value is never an overcount relative to the true max.
    expect(bounded!).toBeLessThanOrEqual(full!);
  });

  it("null propagates identically to the unbounded version for an empty/out-of-range span", () => {
    expect(sampleGridSpanBounded(64, -10, -5, grid)).toBeNull();
    expect(sampleGridSpanBounded(64, 5, 5, grid)).toBeNull();
    expect(sampleGridSpanBounded(0, 0, 10, grid)).toBeNull();
  });
});

describe("pixelDbBounded parity, using SPEC-007 §4.3's real hop-derived time-axis spans", () => {
  it("agrees with an unbounded 2D pixelDb-equivalent across a realistic zoom sweep", () => {
    const totalFrames = 4096;
    const bins = 1025; // FFT 2048
    const lookup = (frame: number, bin: number): number | null =>
      frame < 0 || frame >= totalFrames || bin < 0 || bin >= bins
        ? null
        : Math.sin(frame * 0.05) * 40 + Math.cos(bin * 0.2) * 20 - 60;

    const unboundedPixelDb = (frameLo: number, frameHi: number, binLo: number, binHi: number): number | null => {
      const binValue = (bin: number) => sampleGridSpan(totalFrames, frameLo, frameHi, (f) => lookup(f, bin));
      return sampleGridSpan(bins, binLo, binHi, binValue);
    };

    const rateHz = 48_000;
    const fftSize = autoFftSize(rateHz);
    for (const spp of [0.5, 2, 16, 128, 2048]) {
      const hop = hopForZoom(spp, fftSize);
      const dpr = 1;
      const { lo, hi } = frameColumnBounds(64, 0, spp, dpr, hop);
      // Time-axis span width is constant across columns (SPEC-007 §4.3); well under the 16-tap
      // cap for every zoom level the hop rule produces (1-2 frames/column when zoomed out).
      for (let px = 0; px < 64; px += 8) {
        const frameLo = lo[px]!;
        const frameHi = hi[px]!;
        for (const [binLo, binHi] of [
          [10, 10.5],
          [10, 12],
          [500, 501],
        ] as const) {
          expect(pixelDbBounded(lookup, totalFrames, bins, frameLo, frameHi, binLo, binHi)).toBe(
            unboundedPixelDb(frameLo, frameHi, binLo, binHi),
          );
        }
      }
    }
  });
});

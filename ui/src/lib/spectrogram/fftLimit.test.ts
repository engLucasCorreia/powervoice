import { describe, expect, it } from "vitest";
import { FFT_SIZES } from "./geometry";
import { isFftSizeDisabled } from "./fftLimit";

describe("isFftSizeDisabled (SPEC-007 §2.6, AC-14)", () => {
  it("with MAX_TEXTURE_SIZE stubbed to 8192, only 16384 is disabled", () => {
    const maxTextureSize = 8192;
    const disabled = FFT_SIZES.filter((n) => isFftSizeDisabled(n, maxTextureSize));
    expect(disabled).toEqual([16_384]);
  });

  it("nothing is disabled when the limit is effectively unbounded (Canvas2D)", () => {
    for (const n of FFT_SIZES) {
      expect(isFftSizeDisabled(n, Infinity)).toBe(false);
    }
  });

  it("bin count N/2+1 is the exact threshold", () => {
    expect(isFftSizeDisabled(2048, 1025)).toBe(false); // bins = 1025, exactly at the limit
    expect(isFftSizeDisabled(2048, 1024)).toBe(true); // bins = 1025 > 1024
  });
});

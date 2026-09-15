import { describe, expect, it } from "vitest";
import {
  clampVerticalZoom,
  DEFAULT_VERTICAL_ZOOM,
  MAX_VERTICAL_ZOOM,
  MIN_VERTICAL_ZOOM,
  verticalZoomStep,
  VERTICAL_ZOOM_STEP_FACTOR,
} from "./verticalZoom";

describe("clampVerticalZoom (SPEC-006 §2.4: range 1x..256x)", () => {
  it("passes values already in range through unchanged", () => {
    expect(clampVerticalZoom(1)).toBe(1);
    expect(clampVerticalZoom(64)).toBe(64);
    expect(clampVerticalZoom(256)).toBe(256);
  });

  it("clamps below the floor to MIN_VERTICAL_ZOOM", () => {
    expect(clampVerticalZoom(0)).toBe(MIN_VERTICAL_ZOOM);
    expect(clampVerticalZoom(-5)).toBe(MIN_VERTICAL_ZOOM);
    expect(clampVerticalZoom(0.5)).toBe(MIN_VERTICAL_ZOOM);
  });

  it("clamps above the ceiling to MAX_VERTICAL_ZOOM", () => {
    expect(clampVerticalZoom(257)).toBe(MAX_VERTICAL_ZOOM);
    expect(clampVerticalZoom(1e9)).toBe(MAX_VERTICAL_ZOOM);
  });

  it("falls back to the default for non-finite input", () => {
    expect(clampVerticalZoom(NaN)).toBe(DEFAULT_VERTICAL_ZOOM);
    expect(clampVerticalZoom(Infinity)).toBe(DEFAULT_VERTICAL_ZOOM);
    expect(clampVerticalZoom(-Infinity)).toBe(DEFAULT_VERTICAL_ZOOM);
  });
});

describe("verticalZoomStep (SPEC-006 §2.4: power-of-two steps)", () => {
  it("doubles on zoom-in, halves on zoom-out", () => {
    expect(verticalZoomStep(1, 1)).toBe(VERTICAL_ZOOM_STEP_FACTOR);
    expect(verticalZoomStep(4, 1)).toBe(8);
    expect(verticalZoomStep(4, -1)).toBe(2);
  });

  it("clamps at the floor and ceiling instead of overshooting", () => {
    expect(verticalZoomStep(MIN_VERTICAL_ZOOM, -1)).toBe(MIN_VERTICAL_ZOOM);
    expect(verticalZoomStep(MAX_VERTICAL_ZOOM, 1)).toBe(MAX_VERTICAL_ZOOM);
    // 200 * 2 = 400, clamped to 256.
    expect(verticalZoomStep(200, 1)).toBe(MAX_VERTICAL_ZOOM);
  });

  it("round-trips a step in and back out at the default", () => {
    const zoomedIn = verticalZoomStep(DEFAULT_VERTICAL_ZOOM, 1);
    expect(verticalZoomStep(zoomedIn, -1)).toBe(DEFAULT_VERTICAL_ZOOM);
  });
});

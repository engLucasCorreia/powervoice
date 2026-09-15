import { describe, expect, it } from "vitest";
import { LOOP_STRIP_PX, loopFromRange, loopGeometry } from "./loopOverlay";
import { buildOverlayBatch } from "./overlayGeometry";

/** H-37 (SPEC-006 §2.12 amendment): the loop brace strip and boundary lines. */
describe("loopGeometry", () => {
  it("spans the loop start to end in view pixels, with a line at each end", () => {
    // 10 samples per pixel, view starting at sample 1_000: loop 1_500..3_000 → px 50..200.
    const g = loopGeometry({ startSample: 1_500, endSample: 3_000 }, 1_000, 10, 400);
    expect(g.strip).toEqual({ x0: 50, x1: 200 });
    expect(g.lines).toEqual([50, 200]);
  });

  it("clips the strip to the viewport and drops off-screen boundary lines", () => {
    const g = loopGeometry({ startSample: 0, endSample: 10_000 }, 1_000, 10, 400);
    expect(g.strip).toEqual({ x0: 0, x1: 400 });
    expect(g.lines).toEqual([]);
  });

  it("is empty when the loop is entirely outside the view", () => {
    const g = loopGeometry({ startSample: 0, endSample: 500 }, 1_000, 10, 400);
    expect(g.strip).toBeNull();
    expect(g.lines).toEqual([]);
  });
});

describe("loopFromRange", () => {
  it("maps the engine's loop_range tuple, and null/empty to null", () => {
    expect(loopFromRange([10, 20])).toEqual({ startSample: 10, endSample: 20 });
    expect(loopFromRange(null)).toBeNull();
    expect(loopFromRange(undefined)).toBeNull();
    expect(loopFromRange([20, 20])).toBeNull();
  });
});

describe("buildOverlayBatch with a loop", () => {
  const base = {
    startSample: 0,
    samplesPerPixel: 10,
    viewportPx: 400,
    heightPx: 100,
    selection: null,
    markers: [],
    playheadSample: null,
    colors: {
      selectionFill: [0, 0, 1, 0.2] as const,
      marker: [0, 1, 0, 1] as const,
      markerRegionFill: [0, 1, 0, 0.2] as const,
      playhead: [1, 0.5, 0, 1] as const,
      loop: [0.8, 0.6, 1, 1] as const,
    },
  };

  it("adds the brace strip and two boundary lines only while looping", () => {
    const without = buildOverlayBatch({ ...base, loop: null });
    const withLoop = buildOverlayBatch({ ...base, loop: { startSample: 500, endSample: 2_000 } });
    expect(without.vertexCount).toBe(0);
    // One strip rect + two lines, each a quad of 6 vertices.
    expect(withLoop.vertexCount).toBe(3 * 6);
    expect(LOOP_STRIP_PX).toBeGreaterThan(0);
  });
});

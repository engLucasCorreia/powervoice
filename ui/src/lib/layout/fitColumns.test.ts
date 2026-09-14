import { describe, expect, it } from "vitest";
import { fitSideColumns } from "./fitColumns";

describe("fitSideColumns (H-25: the three columns always fit the window)", () => {
  it("keeps the requested widths when editor + panels fit", () => {
    expect(fitSideColumns({ mainPx: 2126, markersPx: 240, rackPx: 300 })).toEqual({ markersPx: 240, rackPx: 300 });
  });

  it("is a no-op before the main area has been measured", () => {
    expect(fitSideColumns({ mainPx: 0, markersPx: 240, rackPx: 300 })).toEqual({ markersPx: 240, rackPx: 300 });
  });

  it("shrinks both panels proportionally so the editor keeps its minimum", () => {
    // 1000 − 12 splitters − 400 editor = 588 available for 400 + 400 requested.
    const fit = fitSideColumns({ mainPx: 1000, markersPx: 400, rackPx: 400, editorMinPx: 400 });
    expect(fit.markersPx + fit.rackPx).toBeLessThanOrEqual(588);
    expect(fit.markersPx).toBe(294);
    expect(fit.rackPx).toBe(294);
  });

  it("keeps both panels while they stay usable after shrinking", () => {
    // 700 − 12 − 320 = 368 available for 540 requested → 163 + 204, both ≥ 160.
    expect(fitSideColumns({ mainPx: 700, markersPx: 240, rackPx: 300, editorMinPx: 320 })).toEqual({
      markersPx: 163,
      rackPx: 204,
    });
  });

  it("drops Markers before the Rack when both can't stay usable", () => {
    // 600 − 12 − 320 = 268: shrinking would leave Markers at 119 px → Markers hides, Rack gets 268.
    expect(fitSideColumns({ mainPx: 600, markersPx: 240, rackPx: 300, editorMinPx: 320 })).toEqual({
      markersPx: 0,
      rackPx: 268,
    });
  });

  it("narrows the Rack as a last resort, then hides it rather than squeezing the editor", () => {
    const fit = fitSideColumns({ mainPx: 560, markersPx: 240, rackPx: 300, editorMinPx: 320 });
    expect(fit.markersPx).toBe(0);
    expect(fit.rackPx).toBe(228);
    expect(fitSideColumns({ mainPx: 400, markersPx: 240, rackPx: 300, editorMinPx: 320 }).rackPx).toBe(0);
  });

  it("collapsed panels (width 0) stay collapsed and free their space", () => {
    expect(fitSideColumns({ mainPx: 900, markersPx: 0, rackPx: 300, editorMinPx: 320 })).toEqual({
      markersPx: 0,
      rackPx: 300,
    });
  });
});

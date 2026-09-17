import { describe, expect, it } from "vitest";
import { selectionGeometry } from "./selectionOverlay";

/** H-79 (SPEC-006 §2.12 Amendment 2): the selection fill rect and boundary lines. */
describe("selectionGeometry", () => {
  it("spans the selection start to end in view pixels, with a line at each end", () => {
    // 10 samples per pixel, view starting at sample 1_000: selection 1_500..3_000 -> px 50..200.
    const g = selectionGeometry({ startSample: 1_500, endSample: 3_000 }, 1_000, 10, 400);
    expect(g.fill).toEqual({ x0: 50, x1: 200 });
    expect(g.lines).toEqual([50, 200]);
  });

  it("clips the fill to the viewport and drops off-screen boundary lines", () => {
    const g = selectionGeometry({ startSample: 0, endSample: 10_000 }, 1_000, 10, 400);
    expect(g.fill).toEqual({ x0: 0, x1: 400 });
    expect(g.lines).toEqual([]);
  });

  it("is empty when there is no selection", () => {
    const g = selectionGeometry(null, 1_000, 10, 400);
    expect(g.fill).toBeNull();
    expect(g.lines).toEqual([]);
  });

  it("is empty when the selection is entirely outside the view", () => {
    const g = selectionGeometry({ startSample: 0, endSample: 500 }, 1_000, 10, 400);
    expect(g.fill).toBeNull();
    expect(g.lines).toEqual([]);
  });

  it("keeps a boundary line visible just past the edge (1 px tolerance, like loopGeometry)", () => {
    const g = selectionGeometry({ startSample: 1_000, endSample: 1_010 }, 1_000, 10, 400);
    // Start line at px 0 (in view); end line at px 1 (in view) — both well inside the tolerance.
    expect(g.lines).toEqual([0, 1]);
  });
});

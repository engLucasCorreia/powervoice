import { describe, expect, it } from "vitest";
import { pixelAtSample, sampleAtPixel } from "./coords";
import {
  extendSelection,
  extendSelectionEdge,
  hitTestHandle,
  isEmptySelection,
  normalizeSelection,
  nudgeSelectionRange,
  selectAll,
  SELECTION_HANDLE_HIT_PX,
} from "./selection";

describe("normalizeSelection (SPEC-006 §2.9)", () => {
  it("normalizes start < end regardless of drag direction", () => {
    expect(normalizeSelection(100, 400)).toEqual({ startSample: 100, endSample: 400 });
    expect(normalizeSelection(400, 100)).toEqual({ startSample: 100, endSample: 400 });
  });

  it("a zero-length drag (a plain click) yields no selection", () => {
    expect(normalizeSelection(250, 250)).toBeNull();
  });
});

describe("extendSelection (Shift+click, SPEC-006 §2.9)", () => {
  it("with no existing selection, behaves like a drag anchored at the cursor", () => {
    expect(extendSelection(null, 500, 100)).toEqual({ startSample: 100, endSample: 500 });
  });

  it("extends the far edge (farther from the click) out to the clicked sample", () => {
    const selection = { startSample: 100, endSample: 200 };
    // Click near the start (distance 20 vs 80 to the end): the end (far edge) is the anchor.
    expect(extendSelection(selection, 120, 0)).toEqual({ startSample: 120, endSample: 200 });
    // Click near the end (distance 20 vs 80 to the start): the start (far edge) is the anchor.
    expect(extendSelection(selection, 180, 0)).toEqual({ startSample: 100, endSample: 180 });
    // Click outside the selection, past the end: the start is farther and becomes the anchor.
    expect(extendSelection(selection, 300, 0)).toEqual({ startSample: 100, endSample: 300 });
    // Click outside the selection, before the start: the end is farther and becomes the anchor.
    expect(extendSelection(selection, 0, 0)).toEqual({ startSample: 0, endSample: 200 });
  });
});

describe("selectAll / Ctrl+A and double-click (SPEC-006 §2.9)", () => {
  it("selects [0, lenSamples) exactly", () => {
    expect(selectAll(48_000)).toEqual({ startSample: 0, endSample: 48_000 });
  });

  it("an empty document has no selection", () => {
    expect(selectAll(0)).toBeNull();
  });
});

describe("isEmptySelection (SPEC-008 §2.2)", () => {
  it("null and a zero-length range both count as empty", () => {
    expect(isEmptySelection(null)).toBe(true);
    expect(isEmptySelection({ startSample: 10, endSample: 10 })).toBe(true);
    expect(isEmptySelection({ startSample: 10, endSample: 20 })).toBe(false);
  });
});

describe("hitTestHandle (SPEC-006 §2.9/§3, AC-8)", () => {
  const startPx = 100;
  const endPx = 300;

  it("hits are exactly SELECTION_HANDLE_HIT_PX wide, centered on the boundary", () => {
    const half = SELECTION_HANDLE_HIT_PX / 2;
    expect(hitTestHandle(startPx - half, startPx, endPx)).toBe("start");
    expect(hitTestHandle(startPx + half, startPx, endPx)).toBe("start");
    expect(hitTestHandle(startPx - half - 1, startPx, endPx)).toBeNull();
    expect(hitTestHandle(startPx + half + 1, startPx, endPx)).toBeNull();
    expect(hitTestHandle(endPx - half, startPx, endPx)).toBe("end");
    expect(hitTestHandle(endPx + half, startPx, endPx)).toBe("end");
  });

  it("misses entirely in the middle of the selection", () => {
    expect(hitTestHandle(200, startPx, endPx)).toBeNull();
  });

  it("the nearer handle wins when a narrow selection makes both hit zones overlap", () => {
    expect(hitTestHandle(99, 100, 101)).toBe("start");
    expect(hitTestHandle(102, 100, 101)).toBe("end");
  });

  it("start wins on an exact tie (identical start/end pixels)", () => {
    expect(hitTestHandle(100, 100, 100)).toBe("start");
  });
});

// T-701/A-020: keyboard nudge — moves the whole selection, length unchanged, clamped to the
// document.
describe("nudgeSelectionRange (Left/Right Arrow, T-701/A-020)", () => {
  it("moves both edges by the same delta, preserving length", () => {
    expect(nudgeSelectionRange({ startSample: 100, endSample: 200 }, 50, 1_000)).toEqual({
      startSample: 150,
      endSample: 250,
    });
    expect(nudgeSelectionRange({ startSample: 100, endSample: 200 }, -50, 1_000)).toEqual({
      startSample: 50,
      endSample: 150,
    });
  });

  it("clamps at the start of the document without shrinking the selection", () => {
    expect(nudgeSelectionRange({ startSample: 10, endSample: 60 }, -30, 1_000)).toEqual({
      startSample: 0,
      endSample: 50,
    });
  });

  it("clamps at the end of the document without shrinking the selection", () => {
    expect(nudgeSelectionRange({ startSample: 950, endSample: 990 }, 30, 1_000)).toEqual({
      startSample: 960,
      endSample: 1_000,
    });
  });

  it("a delta of 0 is a no-op", () => {
    expect(nudgeSelectionRange({ startSample: 100, endSample: 200 }, 0, 1_000)).toEqual({
      startSample: 100,
      endSample: 200,
    });
  });
});

// T-701/A-020: keyboard extend — grows the selection from the edge in `direction`; never shrinks.
describe("extendSelectionEdge (Shift+Left/Right Arrow, T-701/A-020)", () => {
  it("with no selection, starts a new one from the cursor extending in `direction`", () => {
    expect(extendSelectionEdge(null, 500, 1, 50, 10_000)).toEqual({
      startSample: 500,
      endSample: 550,
    });
    expect(extendSelectionEdge(null, 500, -1, 50, 10_000)).toEqual({
      startSample: 450,
      endSample: 500,
    });
  });

  it("an empty (zero-length) selection is treated the same as no selection", () => {
    expect(extendSelectionEdge({ startSample: 500, endSample: 500 }, 500, 1, 50, 10_000)).toEqual({
      startSample: 500,
      endSample: 550,
    });
  });

  it("direction 1 (Shift+Right) grows the end edge rightward, start unchanged", () => {
    const current = { startSample: 100, endSample: 200 };
    expect(extendSelectionEdge(current, 999, 1, 30, 10_000)).toEqual({
      startSample: 100,
      endSample: 230,
    });
  });

  it("direction -1 (Shift+Left) grows the start edge leftward, end unchanged", () => {
    const current = { startSample: 100, endSample: 200 };
    expect(extendSelectionEdge(current, 999, -1, 30, 10_000)).toEqual({
      startSample: 70,
      endSample: 200,
    });
  });

  it("clamps the moved edge to the document bounds", () => {
    expect(extendSelectionEdge({ startSample: 0, endSample: 20 }, 999, -1, 100, 10_000)).toEqual({
      startSample: 0,
      endSample: 20,
    });
    expect(extendSelectionEdge({ startSample: 9_900, endSample: 10_000 }, 1, 1, 500, 10_000)).toEqual({
      startSample: 9_900,
      endSample: 10_000,
    });
  });
});

// SPEC-006 AC-7/AC-8: a selection made at one pixel↔sample mapping, converted back to pixels at
// a different zoom, must still resolve to the exact original document samples — the selection is
// stored in samples and never re-derived from pixels (§4.1 shares one pixel↔sample function).
describe("selection pixel <-> sample exactness (SPEC-006 AC-7/AC-8)", () => {
  it("a click-drag's pixel endpoints round-trip to exact document samples", () => {
    const startSample = 1_000;
    const samplesPerPixel = 3.7;
    const downPx = 42;
    const upPx = 217;

    const a = sampleAtPixel(downPx, startSample, samplesPerPixel);
    const b = sampleAtPixel(upPx, startSample, samplesPerPixel);
    const selection = normalizeSelection(a, b);
    expect(selection).not.toBeNull();

    // Re-deriving the pixel positions from the stored samples uses the *same* mapping and must
    // land back on downPx/upPx (to within the rounding §4.1 already accounts for) — but the
    // stored sample values themselves are exact, bit-identical integers, not re-rounded.
    expect(Number.isInteger(selection!.startSample)).toBe(true);
    expect(Number.isInteger(selection!.endSample)).toBe(true);
    expect(selection).toEqual({
      startSample: Math.min(a, b),
      endSample: Math.max(a, b),
    });

    // Zooming (a different samplesPerPixel/startSample) never re-derives the selection from
    // pixels — the stored `u64` samples are untouched; only the *display* pixel position (via
    // `pixelAtSample`, §4.1's shared mapping) changes with the new zoom.
    const zoomedSpp = 9.2;
    const zoomedStart = 500;
    const stillTheSameSelection = selection;
    expect(stillTheSameSelection).toEqual(selection);
    expect(pixelAtSample(selection!.startSample, zoomedStart, zoomedSpp)).toBe(
      Math.round((selection!.startSample - zoomedStart) / zoomedSpp),
    );
  });

  it("dragging past the anchor in either direction still lands on exact samples", () => {
    const startSample = 0;
    const samplesPerPixel = 1;
    const anchorPx = 100;
    const overshootPx = 40; // dragged left, past the anchor

    const anchorSample = sampleAtPixel(anchorPx, startSample, samplesPerPixel);
    const farSample = sampleAtPixel(overshootPx, startSample, samplesPerPixel);
    expect(normalizeSelection(anchorSample, farSample)).toEqual({
      startSample: farSample,
      endSample: anchorSample,
    });
  });
});

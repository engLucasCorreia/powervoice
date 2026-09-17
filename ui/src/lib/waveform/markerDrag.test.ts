import { describe, expect, it } from "vitest";
import {
  advanceMarkerAutoscroll,
  dragPointMarker,
  dragRegionEnd,
  dragRegionStart,
  dragRegionWhole,
  DRAG_AUTOSCROLL_RATE_VIEWPORTS_PER_S,
  FLAG_HIT_HEIGHT_PX,
  hitTestMarkerFlag,
  markerAutoscrollDirection,
  markerMagnetTargets,
  snapToMarkerMagnet,
  type DragMarker,
} from "./markerDrag";

// SPEC-009 AC-7's scene: startSample = 0, samplesPerPixel = 200, a 1000 px canvas, a point
// marker at 50 000 (px 250), the cursor at 61 450 and a region [100 000, 110 000).
const START_SAMPLE = 0;
const SPP = 200;
const POINT: DragMarker = { id: 1, pos_samples: 50_000, len_samples: 0 };
const REGION: DragMarker = { id: 2, pos_samples: 100_000, len_samples: 10_000 };
const CURSOR = 61_450;
const DOC_LEN = 480_000;

describe("hitTestMarkerFlag (SPEC-009 §2.5)", () => {
  const markers = [POINT, REGION];

  it("hits a point marker's flag within the hit box, and misses outside it", () => {
    expect(hitTestMarkerFlag(250, 0, markers, START_SAMPLE, SPP)).toEqual({ id: 1, edge: "point" });
    expect(hitTestMarkerFlag(255, 0, markers, START_SAMPLE, SPP)).toEqual({ id: 1, edge: "point" });
    expect(hitTestMarkerFlag(256, 0, markers, START_SAMPLE, SPP)).toBeNull();
    expect(hitTestMarkerFlag(250, FLAG_HIT_HEIGHT_PX, markers, START_SAMPLE, SPP)).toEqual({
      id: 1,
      edge: "point",
    });
    expect(hitTestMarkerFlag(250, FLAG_HIT_HEIGHT_PX + 1, markers, START_SAMPLE, SPP)).toBeNull();
  });

  it("hits a region's start and end flags separately", () => {
    expect(hitTestMarkerFlag(500, 0, markers, START_SAMPLE, SPP)).toEqual({ id: 2, edge: "start" });
    expect(hitTestMarkerFlag(550, 0, markers, START_SAMPLE, SPP)).toEqual({ id: 2, edge: "end" });
  });

  it("the marker line below the flag is not a hit target", () => {
    // Same x as the point's flag, but well below the flag's 12 px height.
    expect(hitTestMarkerFlag(250, 40, markers, START_SAMPLE, SPP)).toBeNull();
  });

  it("at equal distance the earlier sample wins", () => {
    // Two point markers 2 px apart; a hit exactly in the middle is equidistant from both flags.
    const left: DragMarker = { id: 20, pos_samples: 0, len_samples: 0 };
    const right: DragMarker = { id: 21, pos_samples: 2 * SPP, len_samples: 0 };
    expect(hitTestMarkerFlag(1, 0, [left, right], START_SAMPLE, SPP)).toEqual({ id: 20, edge: "point" });
  });
});

describe("snapToMarkerMagnet (SPEC-009 §2.5)", () => {
  it("snaps to the cursor when within the magnet distance", () => {
    // Dragging the point to px 306: raw = 306 * 200 = 61 200; distance to the cursor (61 450) is
    // 250 samples = 1.25 px, within the 6 px magnet — it wins.
    const raw = 306 * SPP;
    const targets = markerMagnetTargets(POINT.id, [POINT, REGION], CURSOR, null);
    expect(snapToMarkerMagnet(raw, targets, SPP)).toBe(61_450);
  });

  it("does not snap when nothing is within range", () => {
    // px 330: raw = 66 000, 22.75 px from the cursor.
    const raw = 330 * SPP;
    const targets = markerMagnetTargets(POINT.id, [POINT, REGION], CURSOR, null);
    expect(snapToMarkerMagnet(raw, targets, SPP)).toBe(66_000);
  });

  it("excludes the dragged marker's own edges but includes every other marker's", () => {
    const targets = markerMagnetTargets(REGION.id, [POINT, REGION], null, null);
    expect(targets).toEqual([50_000]);
  });

  it("includes the selection's two edges", () => {
    const targets = markerMagnetTargets(POINT.id, [POINT], null, { startSample: 1000, endSample: 2000 });
    expect(targets).toEqual([1000, 2000]);
  });

  it("at equal magnet distance the earlier sample wins", () => {
    const targets = [1000, 1100];
    // Raw exactly between them (50 samples = 5 px each way at 10 samples/px, within the 6 px
    // magnet on both sides).
    expect(snapToMarkerMagnet(1050, targets, 10)).toBe(1000);
  });

  it("returns the raw sample unchanged with no targets in range (Alt disables the magnet by never calling this)", () => {
    expect(snapToMarkerMagnet(12_345, [], SPP)).toBe(12_345);
  });
});

describe("drag shape functions (SPEC-009 §2.5, AC-7)", () => {
  it("a point marker's flag moves to the (already-snapped) target, clamped to the document", () => {
    expect(dragPointMarker(61_450, DOC_LEN)).toEqual({ pos_samples: 61_450, len_samples: 0 });
    expect(dragPointMarker(-10, DOC_LEN)).toEqual({ pos_samples: 0, len_samples: 0 });
    expect(dragPointMarker(DOC_LEN + 10, DOC_LEN)).toEqual({ pos_samples: DOC_LEN, len_samples: 0 });
  });

  it("dragging the region's end flag to px 480 clamps to len = 1", () => {
    const raw = 480 * SPP; // 96 000 < pos + 1
    expect(dragRegionEnd(raw, REGION.pos_samples, DOC_LEN)).toEqual({
      pos_samples: 100_000,
      len_samples: 1,
    });
  });

  it("dragging the region's end flag to px 560 gives [100 000, 112 000)", () => {
    const raw = 560 * SPP;
    expect(dragRegionEnd(raw, REGION.pos_samples, DOC_LEN)).toEqual({
      pos_samples: 100_000,
      len_samples: 12_000,
    });
  });

  it("a region's edges can't cross: a start drag stops at end - 1", () => {
    expect(dragRegionStart(999_999, 110_000)).toEqual({ pos_samples: 109_999, len_samples: 1 });
    expect(dragRegionStart(-5, 110_000)).toEqual({ pos_samples: 0, len_samples: 110_000 });
  });

  it("Shift-dragging the region's start flag from px 500 to px 600 gives [120 000, 130 000)", () => {
    // px 500 is exactly the region's start (grab offset 0); px 600 -> raw 120 000.
    const raw = 600 * SPP;
    expect(dragRegionWhole(raw, REGION, "start", DOC_LEN)).toEqual({
      pos_samples: 120_000,
      len_samples: 10_000,
    });
  });

  it("dragRegionWhole clamps the delta at the document edges without changing len", () => {
    expect(dragRegionWhole(-50_000, REGION, "start", DOC_LEN)).toEqual({
      pos_samples: 0,
      len_samples: 10_000,
    });
    expect(dragRegionWhole(DOC_LEN + 50_000, REGION, "end", DOC_LEN)).toEqual({
      pos_samples: DOC_LEN - 10_000,
      len_samples: 10_000,
    });
  });
});

// H-64 (SPEC-009 §2.5/§3 `drag_autoscroll_rate`): auto-scroll while dragging a marker flag past
// the canvas edge.
describe("marker drag auto-scroll (SPEC-009 §2.5/§3)", () => {
  const VIEWPORT_PX = 1000;

  it("markerAutoscrollDirection: 0 inside the canvas, -1/1 beyond its left/right edge", () => {
    expect(markerAutoscrollDirection(0, VIEWPORT_PX)).toBe(0);
    expect(markerAutoscrollDirection(500, VIEWPORT_PX)).toBe(0);
    expect(markerAutoscrollDirection(VIEWPORT_PX, VIEWPORT_PX)).toBe(0); // exactly at the edge
    expect(markerAutoscrollDirection(-1, VIEWPORT_PX)).toBe(-1);
    expect(markerAutoscrollDirection(VIEWPORT_PX + 1, VIEWPORT_PX)).toBe(1);
    expect(markerAutoscrollDirection(-1, 0)).toBe(0); // no known viewport width yet
  });

  it("advances startSample at exactly one viewport width per second", () => {
    // viewportSamples = 1000 px * 200 spp = 200 000; half a second -> half a viewport width.
    const next = advanceMarkerAutoscroll(0, 1, 0.5, SPP, DOC_LEN * 1000, VIEWPORT_PX);
    expect(next).toBe(100_000);
    expect(DRAG_AUTOSCROLL_RATE_VIEWPORTS_PER_S).toBe(1);
  });

  it("scrolls left (negative direction) the same way", () => {
    const start = 150_000;
    const next = advanceMarkerAutoscroll(start, -1, 0.5, SPP, DOC_LEN * 1000, VIEWPORT_PX);
    expect(next).toBe(50_000);
  });

  it("stops exactly at the document's right edge instead of overshooting", () => {
    const lenSamples = 210_000; // maxStart = 210_000 - 200_000 (viewport) = 10_000
    const next = advanceMarkerAutoscroll(0, 1, 10, SPP, lenSamples, VIEWPORT_PX);
    expect(next).toBe(10_000);
    // Continuing to hold beyond the edge never goes past it.
    expect(advanceMarkerAutoscroll(next, 1, 10, SPP, lenSamples, VIEWPORT_PX)).toBe(10_000);
  });

  it("stops exactly at the document's left edge (0), never negative", () => {
    const next = advanceMarkerAutoscroll(5_000, -1, 10, SPP, DOC_LEN * 1000, VIEWPORT_PX);
    expect(next).toBe(0);
  });

  it("is a no-op with no direction, a non-positive dt, or an unknown viewport width", () => {
    expect(advanceMarkerAutoscroll(1_000, 0, 1, SPP, DOC_LEN * 1000, VIEWPORT_PX)).toBe(1_000);
    expect(advanceMarkerAutoscroll(1_000, 1, 0, SPP, DOC_LEN * 1000, VIEWPORT_PX)).toBe(1_000);
    expect(advanceMarkerAutoscroll(1_000, 1, -1, SPP, DOC_LEN * 1000, VIEWPORT_PX)).toBe(1_000);
    expect(advanceMarkerAutoscroll(1_000, 1, 1, SPP, DOC_LEN * 1000, 0)).toBe(1_000);
  });
});

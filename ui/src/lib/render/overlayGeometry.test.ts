import { describe, expect, it } from "vitest";
import { pixelAtSample } from "../waveform/coords";
import { FLOATS_PER_VERTEX } from "./quads";
import { buildOverlayBatch, buildSelectionUnderlay, type OverlayColors } from "./overlayGeometry";

// H-79: selectionFill and selectionBorder are deliberately different colours in these fixtures so
// tests can tell fill-rect vertices apart from boundary-line vertices by their colour channels.
const COLORS: OverlayColors = {
  selectionFill: [0.3, 0.6, 1, 0.22],
  selectionBorder: [0.9, 0.4, 0.8, 1],
  marker: [0.2, 0.77, 0.42, 1],
  markerRegionFill: [0.2, 0.77, 0.42, 0.18],
  playhead: [1, 0.7, 0.33, 1],
};

function xsOf(vertices: Float32Array): number[] {
  const xs: number[] = [];
  for (let i = 0; i < vertices.length; i += FLOATS_PER_VERTEX) {
    xs.push(vertices[i]!);
  }
  return xs;
}

/** x-positions of only the vertices tinted with `color` (H-79: isolates the fill rect from the
 * boundary lines, which share a batch but use different colours) — approximate, since the batch's
 * `Float32Array` rounds each channel from the `number`s in `color`. */
function xsOfColor(vertices: Float32Array, color: readonly [number, number, number, number]): number[] {
  const close = (a: number, b: number) => Math.abs(a - b) < 1e-4;
  const xs: number[] = [];
  for (let i = 0; i < vertices.length; i += FLOATS_PER_VERTEX) {
    if (close(vertices[i + 2]!, color[0]) && close(vertices[i + 3]!, color[1]) && close(vertices[i + 4]!, color[2])) {
      xs.push(vertices[i]!);
    }
  }
  return xs;
}

describe("buildOverlayBatch (H-13, SPEC-006 §4.5 / SPEC-007 §4.7)", () => {
  const base = {
    startSample: 1000,
    samplesPerPixel: 10,
    viewportPx: 200,
    heightPx: 100,
    selection: null,
    markers: [],
    playheadSample: null,
    colors: COLORS,
  };

  it("returns an empty batch with nothing to draw", () => {
    expect(buildOverlayBatch(base).vertexCount).toBe(0);
  });

  it("T-708: marker and playhead lines take the theme's line width (1 px by default)", () => {
    const width = (lineWidthPx?: number) => {
      const xs = xsOf(buildOverlayBatch({ ...base, playheadSample: 1500, lineWidthPx }).toFloat32Array());
      return Math.max(...xs) - Math.min(...xs);
    };
    expect(width()).toBeCloseTo(1, 5);
    expect(width(2)).toBeCloseTo(2, 5);
  });

  it("draws the selection fill at the exact pixelAtSample bounds", () => {
    const batch = buildOverlayBatch({
      ...base,
      selection: { startSample: 1000, endSample: 1500 },
    });
    const xs = xsOfColor(batch.toFloat32Array(), COLORS.selectionFill);
    const expectedX0 = pixelAtSample(1000, base.startSample, base.samplesPerPixel);
    const expectedX1 = pixelAtSample(1500, base.startSample, base.samplesPerPixel);
    expect(Math.min(...xs)).toBe(expectedX0);
    expect(Math.max(...xs)).toBe(expectedX1);
  });

  it("clips the selection fill to the viewport", () => {
    const batch = buildOverlayBatch({
      ...base,
      selection: { startSample: -10_000, endSample: 100_000 },
    });
    const xs = xsOfColor(batch.toFloat32Array(), COLORS.selectionFill);
    expect(Math.min(...xs)).toBe(0);
    expect(Math.max(...xs)).toBe(base.viewportPx);
  });

  it("H-79: also draws a boundary line at each selection edge, in selectionBorder", () => {
    const batch = buildOverlayBatch({
      ...base,
      selection: { startSample: 1000, endSample: 1500 },
    });
    const borderXs = xsOfColor(batch.toFloat32Array(), COLORS.selectionBorder);
    const expectedX0 = pixelAtSample(1000, base.startSample, base.samplesPerPixel);
    const expectedX1 = pixelAtSample(1500, base.startSample, base.samplesPerPixel);
    expect(Math.min(...borderXs)).toBeCloseTo(expectedX0 - 0.5, 5);
    expect(Math.max(...borderXs)).toBeCloseTo(expectedX1 + 0.5, 5);
  });

  it("H-79: omitSelectionFill drops the fill rect but keeps the boundary lines", () => {
    const batch = buildOverlayBatch({
      ...base,
      selection: { startSample: 1000, endSample: 1500 },
      omitSelectionFill: true,
    });
    const vertices = batch.toFloat32Array();
    expect(xsOfColor(vertices, COLORS.selectionFill)).toEqual([]);
    expect(xsOfColor(vertices, COLORS.selectionBorder).length).toBeGreaterThan(0);
  });

  it("H-79: buildSelectionUnderlay draws exactly the fill rect the main batch would omit", () => {
    const selection = { startSample: 1000, endSample: 1500 };
    const underlay = buildSelectionUnderlay({ ...base, selection, color: COLORS.selectionFill });
    const xs = xsOf(underlay.toFloat32Array());
    const expectedX0 = pixelAtSample(1000, base.startSample, base.samplesPerPixel);
    const expectedX1 = pixelAtSample(1500, base.startSample, base.samplesPerPixel);
    expect(Math.min(...xs)).toBe(expectedX0);
    expect(Math.max(...xs)).toBe(expectedX1);
    expect(underlay.vertexCount).toBe(6); // one rect
  });

  it("H-79: buildSelectionUnderlay is empty with no selection", () => {
    expect(buildSelectionUnderlay({ ...base, selection: null, color: COLORS.selectionFill }).vertexCount).toBe(0);
  });

  it("draws the playhead as a thin line at its pixel position", () => {
    const playheadSample = 1000 + 50 * base.samplesPerPixel;
    const batch = buildOverlayBatch({ ...base, playheadSample });
    expect(batch.vertexCount).toBeGreaterThan(0);
    const xs = xsOf(batch.toFloat32Array());
    const px = pixelAtSample(playheadSample, base.startSample, base.samplesPerPixel);
    expect(Math.abs((Math.min(...xs) + Math.max(...xs)) / 2 - px)).toBeLessThan(1);
  });

  it("hides the playhead when off-screen", () => {
    const batch = buildOverlayBatch({ ...base, playheadSample: 1_000_000 });
    expect(batch.vertexCount).toBe(0);
  });

  describe("markerStyle: flags-and-regions (waveform)", () => {
    it("a point marker (len 0) draws just a line + flag, no region", () => {
      const batch = buildOverlayBatch({
        ...base,
        markers: [{ pos_samples: 1000, len_samples: 0 }],
        markerStyle: "flags-and-regions",
      });
      // 1 vLine rect (6 verts) + 1 flag triangle (3 verts) = 9.
      expect(batch.vertexCount).toBe(9);
    });

    it("a region marker (len > 0) also draws a filled band + a second flag", () => {
      const batch = buildOverlayBatch({
        ...base,
        markers: [{ pos_samples: 1000, len_samples: 200 }],
        markerStyle: "flags-and-regions",
      });
      // region rect (6) + end flag (3) + start vLine (6) + start flag (3) = 18.
      expect(batch.vertexCount).toBe(18);
    });
  });

  describe("markerStyle: lines (spectral pane)", () => {
    it("a region marker still draws only its start line — no region fill, no flags", () => {
      const batch = buildOverlayBatch({
        ...base,
        markers: [{ pos_samples: 1000, len_samples: 200 }],
        markerStyle: "lines",
      });
      expect(batch.vertexCount).toBe(6); // exactly one vLine rect.
    });
  });
});

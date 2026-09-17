import { describe, expect, it } from "vitest";
import { FLOATS_PER_VERTEX, VERTICES_PER_QUAD } from "../render/quads";
import { columnYRange, pixelAtSample } from "./coords";
import { buildColumnQuads, buildRawPolyline } from "./webglGeometry";

const COLOR = [0.5, 0.8, 1, 1] as const;

/** Rounds `values` through a `Float32Array` (H-79: matches what a batch's vertices actually hold,
 * so `toEqual` against a plain `number[]` fixture doesn't fail on float32 rounding). */
function f32(values: readonly number[]): number[] {
  return Array.from(new Float32Array(values));
}

describe("buildColumnQuads (H-13, SPEC-006 §2.3/§4.5)", () => {
  it("emits one quad per non-null column, at exactly columnYRange's bounds", () => {
    const columns: Array<[number, number] | null> = [
      [-0.5, 0.5],
      null,
      [0, 1],
    ];
    const centerY = 50;
    const batch = buildColumnQuads(columns, centerY, COLOR);
    expect(batch.vertexCount).toBe(2 * 6); // 2 non-null columns, 6 verts each.
    const v = batch.toFloat32Array();
    const xs: number[] = [];
    const ys: number[] = [];
    for (let i = 0; i < v.length; i += FLOATS_PER_VERTEX) {
      xs.push(v[i]!);
      ys.push(v[i + 1]!);
    }
    // Column 0 (px=0) and column 2 (px=2) are the only ones present.
    expect(Math.min(...xs)).toBe(0);
    expect(Math.max(...xs)).toBe(3); // column 2's right edge is px=3.

    const [top0, bot0] = columnYRange(-0.5, 0.5, centerY);
    const [top2, bot2] = columnYRange(0, 1, centerY);
    expect(Math.min(...ys)).toBe(Math.min(top0, top2));
    expect(Math.max(...ys)).toBe(Math.max(bot0, bot2));
  });

  it("matches columnYRange's minimum-1px-tall guarantee for a silent column", () => {
    const batch = buildColumnQuads([[0, 0]], 40, COLOR);
    const v = batch.toFloat32Array();
    const ys = [];
    for (let i = 0; i < v.length; i += FLOATS_PER_VERTEX) {
      ys.push(v[i + 1]!);
    }
    expect(Math.max(...ys) - Math.min(...ys)).toBe(1);
  });

  it("produces nothing for an all-null column array", () => {
    expect(buildColumnQuads([null, null], 10, COLOR).vertexCount).toBe(0);
  });

  it("verticalZoom (H-35, SPEC-006 §2.4) scales amplitude, matching columnYRange, default 1", () => {
    const centerY = 50;
    const unscaled = buildColumnQuads([[-0.5, 0.5]], centerY, COLOR).toFloat32Array();
    const explicit1 = buildColumnQuads([[-0.5, 0.5]], centerY, COLOR, 1).toFloat32Array();
    expect(Array.from(unscaled)).toEqual(Array.from(explicit1));

    const scaled = buildColumnQuads([[-0.5, 0.5]], centerY, COLOR, 2).toFloat32Array();
    const ys: number[] = [];
    for (let i = 0; i < scaled.length; i += FLOATS_PER_VERTEX) {
      ys.push(scaled[i + 1]!);
    }
    const [top, bottom] = columnYRange(-0.5, 0.5, centerY, 2);
    expect(Math.min(...ys)).toBeCloseTo(top, 5);
    expect(Math.max(...ys)).toBeCloseTo(bottom, 5);
  });
});

describe("buildColumnQuads highlight range (H-79, SPEC-006 §2.12 Amendment 2)", () => {
  const centerY = 50;
  const HIGHLIGHT = [1, 1, 1, 1] as const;

  it("recolors only columns whose pixel falls in [startPx, endPx)", () => {
    // 4 non-null columns -> exactly one quad (6 vertices) each, in order, with no shared/skipped
    // vertices to disambiguate — index by column position rather than by x (column edges are
    // shared between adjacent quads, e.g. column 0's right edge and column 1's left edge are both
    // at x=1, so searching by x alone can't tell which quad's color it belongs to).
    const columns: Array<[number, number] | null> = [[-0.5, 0.5], [-0.5, 0.5], [-0.5, 0.5], [-0.5, 0.5]];
    const batch = buildColumnQuads(columns, centerY, COLOR, 1, undefined, {
      startPx: 1,
      endPx: 3,
      color: HIGHLIGHT,
    });
    const v = batch.toFloat32Array();
    const colorOfColumn = (i: number) => {
      const base = i * VERTICES_PER_QUAD * FLOATS_PER_VERTEX;
      return [v[base + 2], v[base + 3], v[base + 4], v[base + 5]];
    };
    expect(colorOfColumn(0)).toEqual(f32(COLOR));
    expect(colorOfColumn(1)).toEqual(f32(HIGHLIGHT));
    expect(colorOfColumn(2)).toEqual(f32(HIGHLIGHT));
    expect(colorOfColumn(3)).toEqual(f32(COLOR));
  });

  it("without a highlight, every column keeps the base color", () => {
    const batch = buildColumnQuads([[-0.5, 0.5], [-0.5, 0.5]], centerY, COLOR);
    const v = batch.toFloat32Array();
    for (let i = 0; i < v.length; i += FLOATS_PER_VERTEX) {
      expect([v[i + 2], v[i + 3], v[i + 4], v[i + 5]]).toEqual(f32(COLOR));
    }
  });

  it("pending columns are unaffected by a highlight", () => {
    const pending: [number, number] = [Number.NaN, Number.NaN];
    // isPendingColumn checks NaN — see coords.ts; reuse the real pending marker via a NaN pair.
    const batch = buildColumnQuads([pending], centerY, COLOR, 1, [0.2, 0.2, 0.2, 1], {
      startPx: 0,
      endPx: 5,
      color: HIGHLIGHT,
    });
    const v = batch.toFloat32Array();
    expect([v[2], v[3], v[4], v[5]]).toEqual(f32([0.2, 0.2, 0.2, 1]));
  });
});

describe("buildRawPolyline (H-13, SPEC-006 §2.3)", () => {
  const startSample = 1000;
  const samplesPerPixel = 0.25; // raw mode: < 1 sample/px in the other direction (> 1 px/sample)
  const centerY = 50;
  const fetchStartSample = 1000;

  it("builds one widthPx-wide quad per consecutive sample pair, for gl.TRIANGLES (H-31)", () => {
    const samples: Array<[number, number]> = [
      [0.5, 0.5],
      [-0.25, -0.25],
      [0, 0],
    ];
    const widthPx = 1;
    const { line } = buildRawPolyline(samples, fetchStartSample, startSample, samplesPerPixel, centerY, COLOR, false, widthPx);
    // 2 segments (3 points), 1 quad (6 vertices) each.
    expect(line.length).toBe(2 * VERTICES_PER_QUAD * FLOATS_PER_VERTEX);
    // Every vertex's y sits within widthPx/2 of one of the two segment endpoints' y (a hairline
    // quad hugs the polyline it replaces).
    const ys = [0, 1, 2].map((i) => centerY - samples[i]![0] * centerY);
    for (let i = 0; i < line.length; i += FLOATS_PER_VERTEX) {
      const y = line[i + 1]!;
      expect(Math.min(...ys.map((sy) => Math.abs(sy - y)))).toBeLessThanOrEqual(widthPx / 2 + 1e-6);
    }
  });

  it("draws a wider quad for a wider widthPx (High Contrast's heavier stroke, H-31)", () => {
    const samples: Array<[number, number]> = [
      [0, 0],
      [0, 0],
    ];
    const thin = buildRawPolyline(samples, fetchStartSample, startSample, samplesPerPixel, centerY, COLOR, false, 1).line;
    const thick = buildRawPolyline(samples, fetchStartSample, startSample, samplesPerPixel, centerY, COLOR, false, 3).line;
    const spanY = (v: Float32Array) => {
      const ys: number[] = [];
      for (let i = 0; i < v.length; i += FLOATS_PER_VERTEX) {
        ys.push(v[i + 1]!);
      }
      return Math.max(...ys) - Math.min(...ys);
    };
    expect(spanY(thin)).toBeCloseTo(1, 5);
    expect(spanY(thick)).toBeCloseTo(3, 5);
  });

  it("omits dots unless withDots is true", () => {
    const samples: Array<[number, number]> = [
      [0.1, 0.1],
      [0.2, 0.2],
    ];
    const withoutDots = buildRawPolyline(samples, fetchStartSample, startSample, samplesPerPixel, centerY, COLOR, false);
    expect(withoutDots.dots.length).toBe(0);
    const withDots = buildRawPolyline(samples, fetchStartSample, startSample, samplesPerPixel, centerY, COLOR, true);
    expect(withDots.dots.length).toBe(samples.length * FLOATS_PER_VERTEX);
  });

  it("returns an empty line for fewer than 2 samples (nothing to connect)", () => {
    const { line } = buildRawPolyline([[0, 0]], fetchStartSample, startSample, samplesPerPixel, centerY, COLOR, false);
    expect(line.length).toBe(0);
  });

  it("skips undefined gaps in the samples array (one segment, one quad)", () => {
    const samples: Array<[number, number] | undefined> = [[0, 0], undefined, [1, 1]];
    const { line } = buildRawPolyline(samples, fetchStartSample, startSample, samplesPerPixel, centerY, COLOR, false);
    expect(line.length).toBe(VERTICES_PER_QUAD * FLOATS_PER_VERTEX);
  });

  it("verticalZoom (H-35, SPEC-006 §2.4) scales the sample amplitude, default 1", () => {
    const samples: Array<[number, number]> = [
      [0.25, 0.25],
      [-0.25, -0.25],
    ];
    const unscaled = buildRawPolyline(samples, fetchStartSample, startSample, samplesPerPixel, centerY, COLOR, true);
    const explicit1 = buildRawPolyline(samples, fetchStartSample, startSample, samplesPerPixel, centerY, COLOR, true, 1, 1);
    expect(Array.from(unscaled.dots)).toEqual(Array.from(explicit1.dots));

    const scaled = buildRawPolyline(samples, fetchStartSample, startSample, samplesPerPixel, centerY, COLOR, true, 1, 2);
    // Dot y = centerY - sample * verticalZoom * centerY.
    expect(scaled.dots[1]).toBeCloseTo(centerY - 0.25 * 2 * centerY, 5);
    expect(scaled.dots[FLOATS_PER_VERTEX + 1]).toBeCloseTo(centerY - -0.25 * 2 * centerY, 5);
  });

  it("H-79 (SPEC-006 §2.12 Amendment 2): recolors dots/segments inside the highlighted range", () => {
    // px = i * 4 at this samplesPerPixel (0.25): samples land at px 0, 4, 8.
    const samples: Array<[number, number]> = [[0, 0], [0.1, 0.1], [0.2, 0.2]];
    const HIGHLIGHT = [1, 1, 1, 1] as const;
    const { line, dots } = buildRawPolyline(
      samples,
      fetchStartSample,
      startSample,
      samplesPerPixel,
      centerY,
      COLOR,
      true,
      1,
      1,
      { startPx: 4, endPx: 9, color: HIGHLIGHT },
    );
    // Dot 0 (px 0) keeps the base color; dots 1 and 2 (px 4, 8) are highlighted.
    expect([dots[2], dots[3], dots[4], dots[5]]).toEqual(f32(COLOR));
    expect([dots[8], dots[9], dots[10], dots[11]]).toEqual(f32(HIGHLIGHT));
    expect([dots[14], dots[15], dots[16], dots[17]]).toEqual(f32(HIGHLIGHT));
    // The first segment (px 0 -> px 4) takes the endpoint's color per `colorAt(px, ...)` (the
    // segment is colored by its later endpoint, matching how a per-pixel-column boundary works).
    expect([line[2], line[3], line[4], line[5]]).toEqual(f32(HIGHLIGHT));
  });
});

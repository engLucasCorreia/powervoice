import { describe, expect, it } from "vitest";
import { FLOATS_PER_VERTEX, VERTICES_PER_QUAD } from "../render/quads";
import { columnYRange, pixelAtSample } from "./coords";
import { buildColumnQuads, buildRawPolyline } from "./webglGeometry";

const COLOR = [0.5, 0.8, 1, 1] as const;

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
});

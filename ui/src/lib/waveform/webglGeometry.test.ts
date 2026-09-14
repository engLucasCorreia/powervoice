import { describe, expect, it } from "vitest";
import { FLOATS_PER_VERTEX } from "../render/quads";
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
});

describe("buildRawPolyline (H-13, SPEC-006 §2.3)", () => {
  const startSample = 1000;
  const samplesPerPixel = 0.25; // raw mode: < 1 sample/px in the other direction (> 1 px/sample)
  const centerY = 50;
  const fetchStartSample = 1000;

  it("builds a LINE_STRIP vertex per sample at the exact pixelAtSample/centerY position", () => {
    const samples: Array<[number, number]> = [
      [0.5, 0.5],
      [-0.25, -0.25],
      [0, 0],
    ];
    const { line } = buildRawPolyline(samples, fetchStartSample, startSample, samplesPerPixel, centerY, COLOR, false);
    expect(line.length).toBe(samples.length * FLOATS_PER_VERTEX);
    for (let i = 0; i < samples.length; i++) {
      const px = pixelAtSample(fetchStartSample + i, startSample, samplesPerPixel);
      const y = centerY - samples[i]![0] * centerY;
      expect(line[i * FLOATS_PER_VERTEX]).toBeCloseTo(px, 5);
      expect(line[i * FLOATS_PER_VERTEX + 1]).toBeCloseTo(y, 5);
    }
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

  it("skips undefined gaps in the samples array", () => {
    const samples: Array<[number, number] | undefined> = [[0, 0], undefined, [1, 1]];
    const { line } = buildRawPolyline(samples, fetchStartSample, startSample, samplesPerPixel, centerY, COLOR, false);
    expect(line.length).toBe(2 * FLOATS_PER_VERTEX);
  });
});

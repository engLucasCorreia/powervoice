import { describe, expect, it } from "vitest";
import { cssColorToRgba, FLOATS_PER_VERTEX, hexToRgba, QuadBatch, VERTICES_PER_QUAD } from "./quads";

describe("hexToRgba / cssColorToRgba (H-13)", () => {
  it("parses #rrggbb into 0..1 components", () => {
    expect(hexToRgba("#ff8000")).toEqual([1, 128 / 255, 0, 1]);
    expect(hexToRgba("#000000", 0.5)).toEqual([0, 0, 0, 0.5]);
  });

  it("falls back to opaque gray for an unrecognized hex shape", () => {
    expect(hexToRgba("not-a-color")).toEqual([0.5, 0.5, 0.5, 1]);
  });

  it("parses rgba(...) theme tokens (--wave-selection-fill's actual shape)", () => {
    const [r, g, b, a] = cssColorToRgba("rgba(77, 163, 255, 0.22)");
    expect(r).toBeCloseTo(77 / 255, 5);
    expect(g).toBeCloseTo(163 / 255, 5);
    expect(b).toBeCloseTo(255 / 255, 5);
    expect(a).toBeCloseTo(0.22, 5);
  });

  it("parses rgb(...) without an alpha channel, using the fallback alpha", () => {
    const [, , , a] = cssColorToRgba("rgb(10, 20, 30)", 0.75);
    expect(a).toBe(0.75);
  });

  it("falls back to hex parsing for a #rrggbb string", () => {
    expect(cssColorToRgba("#ffffff")).toEqual([1, 1, 1, 1]);
  });
});

describe("QuadBatch (H-13, SPEC-006 §4.5)", () => {
  it("emits 6 vertices per rect, each FLOATS_PER_VERTEX floats", () => {
    const batch = new QuadBatch();
    batch.rect(0, 0, 10, 20, [1, 0, 0, 1]);
    expect(batch.vertexCount).toBe(VERTICES_PER_QUAD);
    expect(batch.toFloat32Array().length).toBe(VERTICES_PER_QUAD * FLOATS_PER_VERTEX);
  });

  it("covers the exact rect bounds (two triangles spanning the corners)", () => {
    const batch = new QuadBatch();
    batch.rect(2, 3, 12, 30, [0.1, 0.2, 0.3, 0.4]);
    const v = batch.toFloat32Array();
    const xs: number[] = [];
    const ys: number[] = [];
    for (let i = 0; i < v.length; i += FLOATS_PER_VERTEX) {
      xs.push(v[i]!);
      ys.push(v[i + 1]!);
      expect(v[i + 2]).toBeCloseTo(0.1, 5);
      expect(v[i + 3]).toBeCloseTo(0.2, 5);
      expect(v[i + 4]).toBeCloseTo(0.3, 5);
      expect(v[i + 5]).toBeCloseTo(0.4, 5);
    }
    expect(Math.min(...xs)).toBe(2);
    expect(Math.max(...xs)).toBe(12);
    expect(Math.min(...ys)).toBe(3);
    expect(Math.max(...ys)).toBe(30);
  });

  it("skips degenerate (empty/inverted) rects", () => {
    const batch = new QuadBatch();
    batch.rect(5, 5, 5, 10, [1, 1, 1, 1]); // zero width
    batch.rect(5, 10, 10, 5, [1, 1, 1, 1]); // inverted height
    expect(batch.vertexCount).toBe(0);
  });

  it("vLine centers a widthPx-wide rect on x", () => {
    const batch = new QuadBatch();
    batch.vLine(10, 0, 100, [1, 1, 1, 1], 2);
    const v = batch.toFloat32Array();
    const xs = [];
    for (let i = 0; i < v.length; i += FLOATS_PER_VERTEX) {
      xs.push(v[i]!);
    }
    expect(Math.min(...xs)).toBe(9);
    expect(Math.max(...xs)).toBe(11);
  });

  it("flag() emits exactly one triangle (3 vertices)", () => {
    const batch = new QuadBatch();
    batch.flag(0, [1, 1, 1, 1]);
    expect(batch.vertexCount).toBe(3);
  });

  it("line() emits a widthPx-wide quad centred on a horizontal segment (H-31)", () => {
    const batch = new QuadBatch();
    batch.line(0, 10, 20, 10, [1, 1, 1, 1], 4);
    expect(batch.vertexCount).toBe(VERTICES_PER_QUAD);
    const v = batch.toFloat32Array();
    const xs: number[] = [];
    const ys: number[] = [];
    for (let i = 0; i < v.length; i += FLOATS_PER_VERTEX) {
      xs.push(v[i]!);
      ys.push(v[i + 1]!);
    }
    expect(Math.min(...xs)).toBeCloseTo(0, 5);
    expect(Math.max(...xs)).toBeCloseTo(20, 5);
    expect(Math.min(...ys)).toBeCloseTo(8, 5); // 10 - widthPx/2
    expect(Math.max(...ys)).toBeCloseTo(12, 5); // 10 + widthPx/2
  });

  it("line() offsets perpendicular to a diagonal segment by exactly widthPx/2", () => {
    const batch = new QuadBatch();
    // A 3-4-5 segment: perpendicular unit vector is (-4/5, 3/5).
    batch.line(0, 0, 3, 4, [0, 0, 0, 1], 10);
    const v = batch.toFloat32Array();
    const corners = new Set<string>();
    for (let i = 0; i < v.length; i += FLOATS_PER_VERTEX) {
      corners.add(`${v[i]!.toFixed(3)},${v[i + 1]!.toFixed(3)}`);
    }
    // hw = 5, normal = (-4/5*5, 3/5*5) = (-4, 3).
    expect(corners.has("-4.000,3.000")).toBe(true);
    expect(corners.has("4.000,-3.000")).toBe(true);
    expect(corners.has("-1.000,7.000")).toBe(true);
    expect(corners.has("7.000,1.000")).toBe(true);
  });

  it("line() skips a zero-length segment", () => {
    const batch = new QuadBatch();
    batch.line(5, 5, 5, 5, [1, 1, 1, 1], 4);
    expect(batch.vertexCount).toBe(0);
  });

  it("append() concatenates another batch's vertices", () => {
    const a = new QuadBatch();
    a.rect(0, 0, 1, 1, [1, 0, 0, 1]);
    const b = new QuadBatch();
    b.rect(1, 1, 2, 2, [0, 1, 0, 1]);
    a.append(b);
    expect(a.vertexCount).toBe(VERTICES_PER_QUAD * 2);
  });
});

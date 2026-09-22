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

/** Rounds a parsed colour back to 0..255 channels (+ alpha to 2 decimals) for readable asserts. */
function bytes(rgba: readonly number[]): number[] {
  return [...rgba.slice(0, 3).map((c) => Math.round(c * 255)), Math.round(rgba[3]! * 100) / 100];
}

describe("cssColorToRgba — every serialization the production CSS minifier emits (H-121)", () => {
  // H-121: `vite build` minifies design-tokens.css, rewriting `rgba(233, 99, 184, 0.28)` into
  // `#e963b847` and `#ffffff` into `#fff`. The parser only knew `#rrggbb`/`rgba(a, b, c, d)`, so in
  // the shipped app every translucent token fell back to OPAQUE mid-gray — the flat gray block the
  // owner saw over the selection in both panes on the WebGL2 renderer.
  it("parses #rrggbbaa (the minified form of every translucent rgba() token)", () => {
    expect(bytes(cssColorToRgba("#e963b847"))).toEqual([233, 99, 184, 0.28]);
    expect(bytes(cssColorToRgba("#4DA3FF38"))).toEqual([77, 163, 255, 0.22]);
  });

  it("parses #rgb and #rgba short hex", () => {
    expect(cssColorToRgba("#fff")).toEqual([1, 1, 1, 1]);
    expect(bytes(cssColorToRgba("#fff3"))).toEqual([255, 255, 255, 0.2]);
    expect(cssColorToRgba("#0000")).toEqual([0, 0, 0, 0]);
    expect(hexToRgba("#f80")).toEqual([1, 136 / 255, 0, 1]);
  });

  it("an explicit alpha argument never overrides alpha the colour itself carries", () => {
    expect(bytes(hexToRgba("#e963b847", 1))).toEqual([233, 99, 184, 0.28]);
    expect(hexToRgba("#ffffff", 0.5)).toEqual([1, 1, 1, 0.5]);
  });

  it("parses the space-separated rgb() syntax with a slash alpha, and percentages", () => {
    expect(bytes(cssColorToRgba("rgb(233 99 184 / 0.28)"))).toEqual([233, 99, 184, 0.28]);
    expect(bytes(cssColorToRgba("rgb(233 99 184 / 28%)"))).toEqual([233, 99, 184, 0.28]);
    expect(bytes(cssColorToRgba("rgba(100%, 0%, 50%, .5)"))).toEqual([255, 0, 128, 0.5]);
    expect(bytes(cssColorToRgba("rgb(233,99,184)"))).toEqual([233, 99, 184, 1]);
  });

  it("parses transparent", () => {
    expect(cssColorToRgba("transparent")).toEqual([0, 0, 0, 0]);
  });

  it("still falls back to opaque gray for something it cannot read", () => {
    expect(cssColorToRgba("color-mix(in srgb, red, blue)")).toEqual([0.5, 0.5, 0.5, 1]);
    expect(cssColorToRgba("#12345")).toEqual([0.5, 0.5, 0.5, 1]);
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

import { describe, expect, it } from "vitest";
import { FLOATS_PER_VERTEX, QuadBatch, type Rgba } from "./quads";

/**
 * T-704: `QuadBatch` writes straight into a growable `Float32Array` (it used to push 36 numbers
 * per quad into a JS array and copy it into a new `Float32Array` every frame — ~30 % of a WebGL2
 * waveform frame at 2126 px, plus GC). The vertices must be exactly what the array version built.
 */
function reference(ops: Array<(push: (...v: number[]) => void) => void>): Float32Array {
  const verts: number[] = [];
  const push = (...v: number[]) => verts.push(...v);
  for (const op of ops) op(push);
  return new Float32Array(verts);
}

const RED: Rgba = [1, 0, 0, 1];
const BLUE: Rgba = [0, 0, 1, 0.5];

describe("QuadBatch on a typed buffer (T-704)", () => {
  it("builds exactly the vertices of the array-backed version, across many capacity doublings", () => {
    const batch = new QuadBatch();
    const ops: Array<(push: (...v: number[]) => void) => void> = [];
    for (let i = 0; i < 5_000; i++) {
      const x = i * 0.5;
      if (i % 3 === 0) {
        batch.rect(x, 1, x + 1, 9 + (i % 7), RED);
        ops.push((p) =>
          p(x, 1, 1, 0, 0, 1, x + 1, 1, 1, 0, 0, 1, x, 9 + (i % 7), 1, 0, 0, 1, x + 1, 1, 1, 0, 0, 1, x + 1, 9 + (i % 7), 1, 0, 0, 1, x, 9 + (i % 7), 1, 0, 0, 1),
        );
      } else if (i % 3 === 1) {
        const [x0, y0, x1, y1, w] = [x, 3, x + 2, 7, 2];
        batch.line(x0, y0, x1, y1, BLUE, w);
        const len = Math.hypot(x1 - x0, y1 - y0);
        const nx = (-(y1 - y0) / len) * (w / 2);
        const ny = ((x1 - x0) / len) * (w / 2);
        ops.push((p) =>
          p(
            x0 + nx, y0 + ny, 0, 0, 1, 0.5,
            x1 + nx, y1 + ny, 0, 0, 1, 0.5,
            x0 - nx, y0 - ny, 0, 0, 1, 0.5,
            x1 + nx, y1 + ny, 0, 0, 1, 0.5,
            x1 - nx, y1 - ny, 0, 0, 1, 0.5,
            x0 - nx, y0 - ny, 0, 0, 1, 0.5,
          ),
        );
      } else {
        batch.flag(x, RED);
        ops.push((p) => p(x, 0, 1, 0, 0, 1, x + 6, 0, 1, 0, 0, 1, x, 8, 1, 0, 0, 1));
      }
    }
    const want = reference(ops);
    const got = batch.toFloat32Array();
    expect(got.length).toBe(want.length);
    expect(Array.from(got)).toEqual(Array.from(want));
    expect(batch.vertexCount).toBe(want.length / FLOATS_PER_VERTEX);
  });

  it("skips degenerate rects and zero-length lines, and appends another batch in place", () => {
    const a = new QuadBatch();
    a.rect(5, 5, 5, 9, RED);
    a.rect(1, 9, 2, 3, RED);
    a.line(3, 3, 3, 3, RED);
    expect(a.vertexCount).toBe(0);
    expect(a.toFloat32Array().length).toBe(0);
    a.rect(0, 0, 1, 1, RED);
    const b = new QuadBatch();
    for (let i = 0; i < 300; i++) b.rect(i, 0, i + 1, 2, BLUE);
    a.append(b);
    a.append(new QuadBatch());
    expect(a.vertexCount).toBe(6 + 300 * 6);
    const all = a.toFloat32Array();
    expect(Array.from(all.subarray(0, 36))).toEqual(Array.from(reference([(p) => p(0, 0, 1, 0, 0, 1, 1, 0, 1, 0, 0, 1, 0, 1, 1, 0, 0, 1, 1, 0, 1, 0, 0, 1, 1, 1, 1, 0, 0, 1, 0, 1, 1, 0, 0, 1)])));
    expect(Array.from(all.subarray(36))).toEqual(Array.from(b.toFloat32Array()));
  });
});

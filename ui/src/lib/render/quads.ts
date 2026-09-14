/**
 * Pixel-space colored-triangle batch builder, shared by the waveform and spectrogram WebGL2
 * renderers (H-13, SPEC-006 §4.5 / SPEC-007 §4.7: "playhead, selection and marker lines are drawn
 * as thin quads in the same draw call batch to avoid extra passes"). Pure and canvas-free — it
 * only appends numbers to a growable array — so it's testable without a GL context, and both
 * renderers share exactly one geometry implementation for these overlays instead of each
 * reimplementing pixel math that could drift from the other or from the Canvas2D fallback.
 *
 * Vertex layout: interleaved `[x, y, r, g, b, a]` in device pixels / unit color, 6 vertices
 * (2 triangles) per quad, 3 vertices per triangle — ready for `gl.drawArrays(gl.TRIANGLES, ...)`.
 */

export type Rgba = readonly [r: number, g: number, b: number, a: number];

export const FLOATS_PER_VERTEX = 6;
export const VERTICES_PER_QUAD = 6;

/** `#rrggbb` (our theme tokens are always this shape) -> `[r, g, b, a]` in `0..1`. Falls back to
 * opaque mid-gray for anything else, so a badly-resolved CSS variable never throws mid-frame. */
export function hexToRgba(hex: string, alpha = 1): Rgba {
  const m = /^#([0-9a-fA-F]{6})$/.exec(hex.trim());
  if (!m) {
    return [0.5, 0.5, 0.5, alpha];
  }
  const n = parseInt(m[1]!, 16);
  return [((n >> 16) & 0xff) / 255, ((n >> 8) & 0xff) / 255, (n & 0xff) / 255, alpha];
}

/** `rgba(r, g, b, a)` (our theme tokens use this shape for translucent fills) -> `[r, g, b, a]` in
 * `0..1`. Falls back like {@link hexToRgba} on a shape it doesn't recognize. */
export function cssColorToRgba(css: string, fallbackAlpha = 1): Rgba {
  const rgbaMatch = /^rgba?\(\s*([\d.]+)\s*,\s*([\d.]+)\s*,\s*([\d.]+)\s*(?:,\s*([\d.]+)\s*)?\)$/.exec(css.trim());
  if (rgbaMatch) {
    const [, r, g, b, a] = rgbaMatch;
    return [Number(r) / 255, Number(g) / 255, Number(b) / 255, a !== undefined ? Number(a) : fallbackAlpha];
  }
  return hexToRgba(css, fallbackAlpha);
}

export class QuadBatch {
  private verts: number[] = [];

  get vertexCount(): number {
    return this.verts.length / FLOATS_PER_VERTEX;
  }

  /** An axis-aligned rectangle `[x0, x1) x [y0, y1)` in device pixels, one solid color. Skips
   * degenerate (empty or inverted) rects so callers don't need to guard first. */
  rect(x0: number, y0: number, x1: number, y1: number, color: Rgba): void {
    if (!(x1 > x0) || !(y1 > y0)) {
      return;
    }
    const [r, g, b, a] = color;
    // Two triangles: (x0,y0)-(x1,y0)-(x0,y1) and (x1,y0)-(x1,y1)-(x0,y1).
    this.verts.push(
      x0, y0, r, g, b, a,
      x1, y0, r, g, b, a,
      x0, y1, r, g, b, a,
      x1, y0, r, g, b, a,
      x1, y1, r, g, b, a,
      x0, y1, r, g, b, a,
    );
  }

  /** A `widthPx`-wide vertical line centred on `x`, spanning `[y0, y1)`. */
  vLine(x: number, y0: number, y1: number, color: Rgba, widthPx = 1): void {
    this.rect(x - widthPx / 2, y0, x + widthPx / 2, y1, color);
  }

  /** A small filled triangle flag (SPEC-006 §2.11), apex down-right from `(x, 0)` — matches the
   * Canvas2D fallback's `drawFlag`. */
  flag(x: number, color: Rgba, width = 6, height = 8): void {
    const [r, g, b, a] = color;
    this.verts.push(x, 0, r, g, b, a, x + width, 0, r, g, b, a, x, height, r, g, b, a);
  }

  /** Appends another batch's vertices in place (for composing sub-builders). */
  append(other: QuadBatch): void {
    this.verts.push(...other.verts);
  }

  toFloat32Array(): Float32Array {
    return new Float32Array(this.verts);
  }
}

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

/** Initial vertex-buffer capacity in floats (64 quads); doubles as needed. */
const INITIAL_FLOATS = 64 * VERTICES_PER_QUAD * FLOATS_PER_VERTEX;

export class QuadBatch {
  // T-704: vertices are written straight into a growable `Float32Array`. The batch used to push
  // 36 numbers per quad into a JS array and copy that into a new `Float32Array` on every frame —
  // ~30 % of a WebGL2 waveform frame at 2126 px (a raw-zoom view is ~100 k segments), plus the GC
  // pauses behind its > 50 ms frames. Same values: a float64 → float32 store rounds exactly like
  // the old `new Float32Array(numbers)` did.
  private buf = new Float32Array(INITIAL_FLOATS);
  private len = 0;

  get vertexCount(): number {
    return this.len / FLOATS_PER_VERTEX;
  }

  /** Room for `floats` more values; returns the (possibly new) buffer. */
  private reserve(floats: number): Float32Array {
    const need = this.len + floats;
    if (need > this.buf.length) {
      let capacity = this.buf.length * 2;
      while (capacity < need) {
        capacity *= 2;
      }
      const next = new Float32Array(capacity);
      next.set(this.buf.subarray(0, this.len));
      this.buf = next;
    }
    return this.buf;
  }

  /** Writes one `[x, y, r, g, b, a]` vertex at float index `i`; returns the next index. */
  private static put(v: Float32Array, i: number, x: number, y: number, color: Rgba): number {
    v[i] = x;
    v[i + 1] = y;
    v[i + 2] = color[0];
    v[i + 3] = color[1];
    v[i + 4] = color[2];
    v[i + 5] = color[3];
    return i + FLOATS_PER_VERTEX;
  }

  /** An axis-aligned rectangle `[x0, x1) x [y0, y1)` in device pixels, one solid color. Skips
   * degenerate (empty or inverted) rects so callers don't need to guard first. */
  rect(x0: number, y0: number, x1: number, y1: number, color: Rgba): void {
    if (!(x1 > x0) || !(y1 > y0)) {
      return;
    }
    const v = this.reserve(VERTICES_PER_QUAD * FLOATS_PER_VERTEX);
    // Two triangles: (x0,y0)-(x1,y0)-(x0,y1) and (x1,y0)-(x1,y1)-(x0,y1).
    let i = this.len;
    i = QuadBatch.put(v, i, x0, y0, color);
    i = QuadBatch.put(v, i, x1, y0, color);
    i = QuadBatch.put(v, i, x0, y1, color);
    i = QuadBatch.put(v, i, x1, y0, color);
    i = QuadBatch.put(v, i, x1, y1, color);
    this.len = QuadBatch.put(v, i, x0, y1, color);
  }

  /** A `widthPx`-wide vertical line centred on `x`, spanning `[y0, y1)`. */
  vLine(x: number, y0: number, y1: number, color: Rgba, widthPx = 1): void {
    this.rect(x - widthPx / 2, y0, x + widthPx / 2, y1, color);
  }

  /** A `widthPx`-wide segment from `(x0, y0)` to `(x1, y1)`, one solid colour — a rectangle
   * centred on the segment and perpendicular to it (two triangles), used to stroke a polyline at
   * an arbitrary width (H-31: WebGL's `LINE_STRIP` can't be widened past 1px in most
   * implementations, so the raw-sample waveform stayed hairline-thin in High Contrast even though
   * every other stroke there is heavier). No joins between segments — acceptable at the pixel
   * spacing this draws at (SPEC-006 §2.3's raw-sample mode). Skips a zero-length segment. */
  line(x0: number, y0: number, x1: number, y1: number, color: Rgba, widthPx = 1): void {
    const dx = x1 - x0;
    const dy = y1 - y0;
    const len = Math.hypot(dx, dy);
    if (len === 0) {
      return;
    }
    const hw = widthPx / 2;
    const nx = (-dy / len) * hw;
    const ny = (dx / len) * hw;
    const v = this.reserve(VERTICES_PER_QUAD * FLOATS_PER_VERTEX);
    let i = this.len;
    i = QuadBatch.put(v, i, x0 + nx, y0 + ny, color);
    i = QuadBatch.put(v, i, x1 + nx, y1 + ny, color);
    i = QuadBatch.put(v, i, x0 - nx, y0 - ny, color);
    i = QuadBatch.put(v, i, x1 + nx, y1 + ny, color);
    i = QuadBatch.put(v, i, x1 - nx, y1 - ny, color);
    this.len = QuadBatch.put(v, i, x0 - nx, y0 - ny, color);
  }

  /** A small filled triangle flag (SPEC-006 §2.11), apex down-right from `(x, 0)` — matches the
   * Canvas2D fallback's `drawFlag`. */
  flag(x: number, color: Rgba, width = 6, height = 8): void {
    const v = this.reserve(3 * FLOATS_PER_VERTEX);
    let i = this.len;
    i = QuadBatch.put(v, i, x, 0, color);
    i = QuadBatch.put(v, i, x + width, 0, color);
    this.len = QuadBatch.put(v, i, x, height, color);
  }

  /** Appends another batch's vertices in place (for composing sub-builders). */
  append(other: QuadBatch): void {
    if (other.len === 0) {
      return;
    }
    this.reserve(other.len).set(other.buf.subarray(0, other.len), this.len);
    this.len += other.len;
  }

  /** The vertices so far — a view into this batch's buffer (no copy), valid until the batch is
   * changed again. Every caller uploads it to GL right away. */
  toFloat32Array(): Float32Array {
    return this.buf.subarray(0, this.len);
  }
}

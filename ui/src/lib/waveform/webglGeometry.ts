/**
 * Pure (canvas-free) WebGL2 vertex-geometry builders for the waveform view (H-13, SPEC-006 §4.5):
 * one (min, max) quad per pixel column, and the raw-sample polyline + dots used below
 * {@link DOT_THRESHOLD_PX_PER_SAMPLE}. Mirrors the Canvas2D fallback's `drawColumns`/
 * `drawRawPolyline` pixel math exactly (both call the same `columnYRange`/`pixelAtSample` from
 * `coords.ts`), so the two renderers are guaranteed to agree pixel-for-pixel rather than by
 * coincidence — testable here without a GL context, unlike the shader/draw-call code in
 * `webglRenderer.ts`.
 */

import { columnYRange, pixelAtSample } from "./coords";
import { QuadBatch, type Rgba } from "../render/quads";

/** One quad per non-`null` column (SPEC-006 §2.3 min/max fill), in the same left-to-right order
 * as {@link reduceColumns}'s output. */
export function buildColumnQuads(
  columns: ReadonlyArray<[number, number] | null>,
  centerY: number,
  color: Rgba,
): QuadBatch {
  const batch = new QuadBatch();
  for (let px = 0; px < columns.length; px++) {
    const column = columns[px];
    if (!column) {
      continue;
    }
    const [mn, mx] = column;
    const [yTop, yBot] = columnYRange(mn, mx, centerY);
    batch.rect(px, yTop, px + 1, yBot, color);
  }
  return batch;
}

export interface RawPolylineGeometry {
  /** `[x, y, r, g, b, a]` x6 per segment (2 triangles), for `gl.TRIANGLES` — one `widthPx`-wide
   * quad per consecutive sample pair (H-31: WebGL can't widen a `LINE_STRIP` past 1px in most
   * implementations, so the raw-sample line stayed hairline-thin even in High Contrast, unlike
   * the Canvas2D fallback's `ctx.lineWidth`-stroked path). Empty when there are fewer than 2
   * samples (nothing to connect). */
  line: Float32Array;
  /** `[x, y, r, g, b, a]` per vertex, for `gl.POINTS` (SPEC-006 §2.3's raw-sample dots) — empty
   * unless {@link showsDots} would return `true` at the caller's `samplesPerPixel`. */
  dots: Float32Array;
}

/** The raw polyline + dots (SPEC-006 §2.3, below `RAW_SPP`), built with the exact same
 * `pixelAtSample`/centerY math as the Canvas2D fallback's `drawRawPolyline`. `widthPx` matches the
 * theme's stroke width (`themeColors().strokePx`) so the two renderers agree pixel-for-pixel,
 * including in High Contrast's heavier stroke (H-31). */
export function buildRawPolyline(
  samples: ReadonlyArray<readonly [number, number] | undefined>,
  fetchStartSample: number,
  startSample: number,
  samplesPerPixel: number,
  centerY: number,
  color: Rgba,
  withDots: boolean,
  widthPx = 1,
): RawPolylineGeometry {
  const [r, g, b, a] = color;
  const batch = new QuadBatch();
  const dotPts: number[] = [];
  let prevPx: number | null = null;
  let prevY: number | null = null;
  let pointCount = 0;
  for (let i = 0; i < samples.length; i++) {
    const sample = samples[i];
    if (!sample) {
      continue;
    }
    const px = pixelAtSample(fetchStartSample + i, startSample, samplesPerPixel);
    const y = centerY - sample[0] * centerY;
    if (withDots) {
      dotPts.push(px, y, r, g, b, a);
    }
    if (prevPx !== null && prevY !== null) {
      batch.line(prevPx, prevY, px, y, color, widthPx);
    }
    prevPx = px;
    prevY = y;
    pointCount++;
  }
  return {
    line: pointCount >= 2 ? batch.toFloat32Array() : new Float32Array(0),
    dots: new Float32Array(dotPts),
  };
}

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
  /** `[x, y, r, g, b, a]` per vertex, for `gl.LINE_STRIP` (one point per raw sample). Empty when
   * there are fewer than 2 samples (nothing to connect). */
  line: Float32Array;
  /** `[x, y, r, g, b, a]` per vertex, for `gl.POINTS` (SPEC-006 §2.3's raw-sample dots) — empty
   * unless {@link showsDots} would return `true` at the caller's `samplesPerPixel`. */
  dots: Float32Array;
}

/** The raw polyline + dots (SPEC-006 §2.3, below `RAW_SPP`), built with the exact same
 * `pixelAtSample`/centerY math as the Canvas2D fallback's `drawRawPolyline`. */
export function buildRawPolyline(
  samples: ReadonlyArray<readonly [number, number] | undefined>,
  fetchStartSample: number,
  startSample: number,
  samplesPerPixel: number,
  centerY: number,
  color: Rgba,
  withDots: boolean,
): RawPolylineGeometry {
  const [r, g, b, a] = color;
  const linePts: number[] = [];
  for (let i = 0; i < samples.length; i++) {
    const sample = samples[i];
    if (!sample) {
      continue;
    }
    const px = pixelAtSample(fetchStartSample + i, startSample, samplesPerPixel);
    const y = centerY - sample[0] * centerY;
    linePts.push(px, y, r, g, b, a);
  }
  const dotPts: number[] = [];
  if (withDots) {
    dotPts.push(...linePts);
  }
  return {
    line: linePts.length >= 12 ? new Float32Array(linePts) : new Float32Array(0),
    dots: new Float32Array(dotPts),
  };
}
